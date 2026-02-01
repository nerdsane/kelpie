//! Kelpie Server Library
//!
//! This crate provides the Kelpie server components for building
//! Letta-compatible AI agent systems.

extern crate self as kelpie_server;

pub mod actor;
pub mod api;
pub mod http;
pub mod interface;
pub mod llm;
pub mod memory;
pub mod models;
pub mod security;
pub mod service;
pub mod state;
pub mod storage;
pub mod tools;
