# test-failures

**Examined:** 2026-01-24T21:07:06.640418+00:00

## Summary

11 test files fail to compile due to deleted AgentService methods

## Connections

- kelpie-server

## Details

The AgentService struct no longer has `new_without_wal()` and `recover()` methods, but 11 test files still reference them:

**Compilation Errors:**
1. `AgentService::new_without_wal(handle)` - method removed but used in 10 files
2. `service.recover().await` - method removed, used in agent_deactivation_timing.rs

**Affected test files:**
- runtime_pilot_test.rs (new_without_wal at line 77)
- delete_atomicity_test.rs (new_without_wal at line 370)
- agent_deactivation_timing.rs (both new_without_wal at line 494, and recover() at line 85)
- real_llm_integration.rs (new_without_wal at line 85)
- agent_service_fault_injection.rs (new_without_wal at line 681)
- appstate_integration_dst.rs
- agent_service_dst.rs
- agent_message_handling_dst.rs
- agent_streaming_dst.rs
- llm_token_streaming_dst.rs
- agent_service_send_message_full_dst.rs

**Current implementation:**
AgentService only has `new(dispatcher: DispatcherHandle<R>)` constructor.

**Fix needed:**
Replace all `AgentService::new_without_wal(handle)` with `AgentService::new(handle)`.
Remove or replace all `service.recover()` calls.

## Issues

### [CRITICAL] 11 test files fail to compile due to deleted AgentService::new_without_wal() method

**Evidence:** cargo test shows E0599 errors in runtime_pilot_test.rs:77, delete_atomicity_test.rs:370, agent_deactivation_timing.rs:494, real_llm_integration.rs:85, agent_service_fault_injection.rs:681, plus 6 more DST test files

### [HIGH] agent_deactivation_timing.rs calls deleted recover() method

**Evidence:** E0599: no method named recover found at line 85
