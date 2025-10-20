//! # agpod
//!
//! A powerful agent helper tool with features including git diff minimization
//! for LLM context and PR draft management.
//!
//! ## Modules
//!
//! - `config`: Configuration management for all features
//! - `diff`: Git diff minimization and processing
//! - `kiro`: PR draft workflow management

pub mod config;
pub mod diff;
pub mod kiro;
