//! RytherVM - EVM execution engine.
//!
//! Provides high-level EVM execution interface with:
//! - Transaction execution
//! - Gas metering
//! - State change tracking
//!
//! This is a simplified implementation that demonstrates the architecture.
//! Production use would integrate fully with revm.

use std::collections::HashMap;
use thiserror::Error;

use crate::execution::mvcc::MultiVersionState;
use crate::types::state::{ExecutionResult, ExecutionStatus, Log, ReadSet, WriteSet};
use crate::types::transaction::DecryptedTransaction;
use crate::types::{keccak256, Address, StateKey, U256};

/// State slot for account balance.
pub const SLOT_BALANCE: u64 = 0;
/// State slot for account nonce.
pub const SLOT_NONCE: u64 = 1;
/// State slot for code hash.
pub const SLOT_CODE_HASH: u64 = 2;

/// EVM execution errors.
#[derive(Debug, Error)]
pub enum VmError {
    #[error("Insufficient balance")]
    InsufficientBalance,

    #[error("Invalid nonce: expected {expected}, got {got}")]
    InvalidNonce { expected: u64, got: u64 },

    #[error("Out of gas")]
    OutOfGas,

    #[error("Invalid opcode: {0}")]
    InvalidOpcode(u8),

    #[error("Stack underflow")]
    StackUnderflow,

    #[error("Stack overflow")]
    StackOverflow,

    #[error("Invalid jump destination")]
    InvalidJump,

    #[error("Revert: {0}")]
    Revert(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result of VM execution.
#[derive(Debug)]
pub struct VmResult {
    /// Success or failure
    pub success: bool,

    /// Gas used
    pub gas_used: u64,

    /// Return data
    pub output: Vec<u8>,

    /// Emitted logs
    pub logs: Vec<Log>,

    /// Error message if failed
    pub error: Option<String>,
}

/// Block context for EVM execution.
#[derive(Clone, Debug)]
pub struct BlockContext {
    pub number: u64,
    pub timestamp: u64,
    pub coinbase: Address,
    pub gas_limit: u64,
    pub base_fee: U256,
    pub chain_id: u64,
}

impl Default for BlockContext {
    fn default() -> Self {
        Self {
            number: 0,
            timestamp: 0,
            coinbase: Address::zero(),
            gas_limit: 30_000_000,
            base_fee: U256::from_u64(1_000_000_000), // 1 gwei
            chain_id: 1,
        }
    }
}

/// RytherVM - EVM execution engine.
pub struct RytherVm {
    /// Chain ID
    chain_id: u64,

    /// Contract code storage
    code_storage: HashMap<Address, Vec<u8>>,
}

impl RytherVm {
    /// Create a new RytherVM instance.
    pub fn new(chain_id: u64) -> Self {
        Self {
            chain_id,
            code_storage: HashMap::new(),
        }
    }

    /// Deploy contract code.
    pub fn deploy_code(&mut self, address: Address, code: Vec<u8>) {
        self.code_storage.insert(address, code);
    }

    /// Get contract code.
    pub fn get_code(&self, address: &Address) -> Option<&Vec<u8>> {
        self.code_storage.get(address)
    }

    /// Execute a transaction.
    pub fn execute(
        &self,
        tx: &DecryptedTransaction,
        state: &MultiVersionState,
        block: &BlockContext,
    ) -> ExecutionResult {
        let mut read_set = ReadSet::new();
        let mut write_set = WriteSet::new();

        // Validate and execute transaction
        let result = self.execute_inner(tx, state, block, &mut read_set, &mut write_set);

        // Build execution result
        match result {
            Ok(vm_result) => ExecutionResult {
                tx_seq: tx.sequence_number,
                status: if vm_result.success {
                    ExecutionStatus::Success
                } else {
                    ExecutionStatus::Revert
                },
                read_set,
                write_set,
                gas_used: vm_result.gas_used,
                logs: vm_result.logs,
                output: vm_result.output,
            },
            Err(e) => ExecutionResult {
                tx_seq: tx.sequence_number,
                status: ExecutionStatus::Failure,
                read_set,
                write_set,
                gas_used: tx.gas_limit,
                logs: vec![],
                output: format!("{}", e).into_bytes(),
            },
        }
    }

    /// Internal execution logic.
    fn execute_inner(
        &self,
        tx: &DecryptedTransaction,
        state: &MultiVersionState,
        block: &BlockContext,
        read_set: &mut ReadSet,
        write_set: &mut WriteSet,
    ) -> Result<VmResult, VmError> {
        // Read sender state
        let sender_balance = self.read_balance(state, &tx.from, tx.sequence_number, read_set);
        let sender_nonce = self.read_nonce(state, &tx.from, tx.sequence_number, read_set);

        // Validate nonce
        if sender_nonce != tx.nonce {
            return Err(VmError::InvalidNonce {
                expected: sender_nonce,
                got: tx.nonce,
            });
        }

        // Calculate gas cost
        let gas_cost = self.calculate_gas_cost(tx);
        let total_cost = self.add_u256(&tx.value, &gas_cost);

        // Check balance
        if self.compare_u256(&sender_balance, &total_cost) < 0 {
            return Err(VmError::InsufficientBalance);
        }

        // Deduct gas cost from sender
        let new_sender_balance = self.sub_u256(&sender_balance, &total_cost);
        self.write_balance(
            state,
            &tx.from,
            tx.sequence_number,
            new_sender_balance,
            write_set,
        );

        // Increment sender nonce
        self.write_nonce(
            state,
            &tx.from,
            tx.sequence_number,
            sender_nonce + 1,
            write_set,
        );

        let (success, gas_used, output, logs) = if let Some(to) = tx.to {
            // Call transaction
            self.execute_call(tx, state, &to, read_set, write_set)?
        } else {
            // Create transaction
            self.execute_create(tx, state, read_set, write_set)?
        };

        // Refund unused gas
        let gas_refund = (tx.gas_limit - gas_used) * self.u256_to_u64(&tx.gas_price);
        let refund_amount = U256::from_u64(gas_refund);
        let final_balance = self.add_u256(&new_sender_balance, &refund_amount);
        self.write_balance(
            state,
            &tx.from,
            tx.sequence_number,
            final_balance,
            write_set,
        );

        // Pay coinbase
        let coinbase_balance =
            self.read_balance(state, &block.coinbase, tx.sequence_number, read_set);
        let coinbase_fee = U256::from_u64(gas_used * self.u256_to_u64(&tx.gas_price));
        let new_coinbase_balance = self.add_u256(&coinbase_balance, &coinbase_fee);
        self.write_balance(
            state,
            &block.coinbase,
            tx.sequence_number,
            new_coinbase_balance,
            write_set,
        );

        Ok(VmResult {
            success,
            gas_used,
            output,
            logs,
            error: None,
        })
    }

    /// Execute a call to existing contract.
    fn execute_call(
        &self,
        tx: &DecryptedTransaction,
        state: &MultiVersionState,
        to: &Address,
        read_set: &mut ReadSet,
        write_set: &mut WriteSet,
    ) -> Result<(bool, u64, Vec<u8>, Vec<Log>), VmError> {
        // Transfer value
        if tx.value != U256::ZERO {
            let to_balance = self.read_balance(state, to, tx.sequence_number, read_set);
            let new_to_balance = self.add_u256(&to_balance, &tx.value);
            self.write_balance(state, to, tx.sequence_number, new_to_balance, write_set);
        }

        // Check if contract has code
        if let Some(code) = self.code_storage.get(to) {
            if !code.is_empty() {
                // Execute contract code
                // For now, just consume gas and return success
                let gas_used = 21000 + tx.data.len() as u64 * 16;
                return Ok((true, gas_used.min(tx.gas_limit), vec![], vec![]));
            }
        }

        // Simple transfer (no code)
        Ok((true, 21000, vec![], vec![]))
    }

    /// Execute contract creation.
    fn execute_create(
        &self,
        tx: &DecryptedTransaction,
        state: &MultiVersionState,
        read_set: &mut ReadSet,
        write_set: &mut WriteSet,
    ) -> Result<(bool, u64, Vec<u8>, Vec<Log>), VmError> {
        // Compute new contract address
        let contract_address = self.compute_create_address(&tx.from, tx.nonce);

        // Transfer value to new contract
        if tx.value != U256::ZERO {
            self.write_balance(
                state,
                &contract_address,
                tx.sequence_number,
                tx.value,
                write_set,
            );
        }

        // Store code hash
        let code_hash = keccak256(&tx.data);
        let code_hash_key = StateKey {
            address: contract_address.0,
            slot: U256::from_u64(SLOT_CODE_HASH),
        };
        state.write(
            code_hash_key,
            tx.sequence_number,
            Some(U256::from_be_bytes(code_hash)),
        );

        // Gas for creation
        let gas_used = 32000 + tx.data.len() as u64 * 200;

        Ok((
            true,
            gas_used.min(tx.gas_limit),
            contract_address.0.to_vec(),
            vec![],
        ))
    }

    /// Compute CREATE address.
    fn compute_create_address(&self, sender: &Address, nonce: u64) -> Address {
        let mut data = Vec::new();
        data.push(0xd6); // RLP prefix
        data.push(0x94); // Address length
        data.extend_from_slice(&sender.0);

        // RLP encode nonce
        if nonce == 0 {
            data.push(0x80);
        } else if nonce < 128 {
            data.push(nonce as u8);
        } else {
            let nonce_bytes = nonce.to_be_bytes();
            let leading_zeros = nonce_bytes.iter().take_while(|&&b| b == 0).count();
            let nonce_bytes = &nonce_bytes[leading_zeros..];
            data.push(0x80 + nonce_bytes.len() as u8);
            data.extend_from_slice(nonce_bytes);
        }

        let hash = keccak256(&data);
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&hash[12..32]);
        Address(addr)
    }

    // State access helpers

    fn balance_key(address: &Address) -> StateKey {
        StateKey {
            address: address.0,
            slot: U256::from_u64(SLOT_BALANCE),
        }
    }

    fn nonce_key(address: &Address) -> StateKey {
        StateKey {
            address: address.0,
            slot: U256::from_u64(SLOT_NONCE),
        }
    }

    fn read_balance(
        &self,
        state: &MultiVersionState,
        addr: &Address,
        seq: u64,
        read_set: &mut ReadSet,
    ) -> U256 {
        state
            .read_tracked(&Self::balance_key(addr), seq, read_set)
            .unwrap_or(U256::ZERO)
    }

    fn read_nonce(
        &self,
        state: &MultiVersionState,
        addr: &Address,
        seq: u64,
        read_set: &mut ReadSet,
    ) -> u64 {
        state
            .read_tracked(&Self::nonce_key(addr), seq, read_set)
            .map(|v| v.0[0])
            .unwrap_or(0)
    }

    fn write_balance(
        &self,
        state: &MultiVersionState,
        addr: &Address,
        seq: u64,
        value: U256,
        write_set: &mut WriteSet,
    ) {
        state.write_tracked(Self::balance_key(addr), seq, Some(value), write_set);
    }

    fn write_nonce(
        &self,
        state: &MultiVersionState,
        addr: &Address,
        seq: u64,
        nonce: u64,
        write_set: &mut WriteSet,
    ) {
        state.write_tracked(
            Self::nonce_key(addr),
            seq,
            Some(U256::from_u64(nonce)),
            write_set,
        );
    }

    // U256 helpers

    fn calculate_gas_cost(&self, tx: &DecryptedTransaction) -> U256 {
        let gas_cost = tx.gas_limit * self.u256_to_u64(&tx.gas_price);
        U256::from_u64(gas_cost)
    }

    fn u256_to_u64(&self, val: &U256) -> u64 {
        val.0[0]
    }

    fn add_u256(&self, a: &U256, b: &U256) -> U256 {
        a.wrapping_add(*b)
    }

    fn sub_u256(&self, a: &U256, b: &U256) -> U256 {
        // Simple subtraction for lower limb
        let mut result = *a;
        let (diff, borrow) = a.0[0].overflowing_sub(b.0[0]);
        result.0[0] = diff;
        if borrow && a.0[1] > 0 {
            result.0[1] -= 1;
        }
        result
    }

    fn compare_u256(&self, a: &U256, b: &U256) -> i32 {
        for i in (0..4).rev() {
            if a.0[i] > b.0[i] {
                return 1;
            }
            if a.0[i] < b.0[i] {
                return -1;
            }
        }
        0
    }
}

impl Default for RytherVm {
    fn default() -> Self {
        Self::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tx(from: u8, to: Option<u8>, value: u64, nonce: u64, seq: u64) -> DecryptedTransaction {
        DecryptedTransaction {
            from: Address([from; 20]),
            to: to.map(|t| Address([t; 20])),
            value: U256::from_u64(value),
            gas_limit: 100000,
            gas_price: U256::from_u64(1_000_000_000),
            nonce,
            data: vec![],
            source_commitment: [0; 32],
            sequence_number: seq,
        }
    }

    #[test]
    fn test_vm_creation() {
        let vm = RytherVm::new(1);
        assert_eq!(vm.chain_id, 1);
    }

    #[test]
    fn test_simple_transfer() {
        let vm = RytherVm::new(1);
        let state = MultiVersionState::new();
        let block = BlockContext::default();

        // Fund sender
        let sender = Address([0xAA; 20]);
        let sender_balance_key = RytherVm::balance_key(&sender);
        state.write(
            sender_balance_key,
            0,
            Some(U256::from_u64(1_000_000_000_000_000_000)),
        ); // 1 ETH
        state.mark_committed(&sender_balance_key, 0);

        // Create transfer
        let tx = make_tx(0xAA, Some(0xBB), 100_000_000_000_000_000, 0, 1); // 0.1 ETH

        let result = vm.execute(&tx, &state, &block);

        assert_eq!(result.status, ExecutionStatus::Success);
        assert!(result.gas_used >= 21000);
    }

    #[test]
    fn test_insufficient_balance() {
        let vm = RytherVm::new(1);
        let state = MultiVersionState::new();
        let block = BlockContext::default();

        // No funding - sender has 0 balance
        let tx = make_tx(0xAA, Some(0xBB), 100_000_000_000_000_000, 0, 1);

        let result = vm.execute(&tx, &state, &block);

        assert_eq!(result.status, ExecutionStatus::Failure);
    }

    #[test]
    fn test_invalid_nonce() {
        let vm = RytherVm::new(1);
        let state = MultiVersionState::new();
        let block = BlockContext::default();

        // Fund sender
        let sender = Address([0xAA; 20]);
        let sender_balance_key = RytherVm::balance_key(&sender);
        state.write(
            sender_balance_key,
            0,
            Some(U256::from_u64(1_000_000_000_000_000_000)),
        );
        state.mark_committed(&sender_balance_key, 0);

        // Wrong nonce (expected 0, got 5)
        let tx = make_tx(0xAA, Some(0xBB), 100_000_000, 5, 1);

        let result = vm.execute(&tx, &state, &block);

        assert_eq!(result.status, ExecutionStatus::Failure);
    }

    #[test]
    fn test_contract_creation() {
        let vm = RytherVm::new(1);
        let state = MultiVersionState::new();
        let block = BlockContext::default();

        // Fund sender
        let sender = Address([0xAA; 20]);
        let sender_balance_key = RytherVm::balance_key(&sender);
        state.write(
            sender_balance_key,
            0,
            Some(U256::from_u64(1_000_000_000_000_000_000)),
        );
        state.mark_committed(&sender_balance_key, 0);

        // Create contract (to = None)
        let mut tx = make_tx(0xAA, None, 0, 0, 1);
        tx.data = vec![0x60, 0x00, 0x60, 0x00, 0xf3]; // Simple bytecode

        let result = vm.execute(&tx, &state, &block);

        assert_eq!(result.status, ExecutionStatus::Success);
        assert!(!result.output.is_empty()); // Should return contract address
    }

    #[test]
    fn test_create_address_computation() {
        let vm = RytherVm::new(1);

        let sender = Address([0x00; 20]);
        let addr0 = vm.compute_create_address(&sender, 0);
        let addr1 = vm.compute_create_address(&sender, 1);

        // Different nonces should produce different addresses
        assert_ne!(addr0, addr1);

        // Same inputs should produce same address
        let addr0_again = vm.compute_create_address(&sender, 0);
        assert_eq!(addr0, addr0_again);
    }

    #[test]
    fn test_read_write_tracking() {
        let vm = RytherVm::new(1);
        let state = MultiVersionState::new();
        let block = BlockContext::default();

        // Fund sender
        let sender = Address([0xAA; 20]);
        let sender_balance_key = RytherVm::balance_key(&sender);
        state.write(
            sender_balance_key,
            0,
            Some(U256::from_u64(1_000_000_000_000_000_000)),
        );
        state.mark_committed(&sender_balance_key, 0);

        let tx = make_tx(0xAA, Some(0xBB), 100_000_000, 0, 1);
        let result = vm.execute(&tx, &state, &block);

        // Should have tracked reads (sender balance, nonce) and writes
        assert!(!result.read_set.is_empty());
        assert!(!result.write_set.is_empty());
    }
}
