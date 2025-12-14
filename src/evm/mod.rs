//! RytherVM EVM Execution Engine.
//!
//! Provides EVM bytecode execution with MVCC state integration.
//! This module provides the interface - full revm integration
//! requires matching the specific revm version APIs.

pub mod vm;

pub use vm::{RytherVm, VmResult, VmError};
