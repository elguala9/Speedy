# Windows Firewall & TCP Loopback — Known Issue

## Symptom

`TcpStream::connect("127.0.0.1:<port>")` **blocks for 20+ seconds** instead of returning immediately with `ConnectionRefused` when nothing is listening on the port.

## Root Cause

On a clean Windows stack, connecting to a port without a listener produces an **RST** packet immediately → `connect()` fails fast.

However, **Windows Defender Firewall / Windows Filtering Platform (WFP) drivers** (common with antivirus, VPNs, corporate security software) can **silently drop the SYN** packet on loopback interfaces without sending RST.

When the SYN is dropped:
- The client (`TcpStream::connect`) never gets a response
- TCP retransmits after ~3s, ~6s, ~12s...
- Default timeout is **20–30 seconds**
- Eventually the connect fails, but far from "instant"

## Where It Hits

```rust
// packages/speedy-core/src/daemon_client.rs
pub async fn is_alive(&self) -> bool {
    TcpStream::connect(&self.addr).await.is_ok()  // ← BLOCKS if no listener
}
```

Called from:
- `speedy/src/main.rs` — `ensure_daemon()` line 69
- `speedy-cli/src/main.rs` — `ensure_daemon()` line 103

Any code path that calls `is_alive()` or `DaemonClient::cmd()` when the daemon **is not running** will block for 20–30s on affected Windows machines.

## Workarounds

### 1. Add connect timeout (recommended)

Replace `TcpStream::connect` with a timed version:

```rust
use tokio::time::timeout;
use tokio::net::TcpStream;

pub async fn is_alive(&self) -> bool {
    timeout(Duration::from_secs(2), TcpStream::connect(&self.addr))
        .await
        .ok()
        .and_then(|r| r.ok())
        .is_some()
}
```

### 2. Verify with `NETSH` / `Test-NetConnection`

```powershell
# Check if firewall is dropping SYNs
Test-NetConnection -ComputerName 127.0.0.1 -Port 42137

# Reset Winsock if the stack is corrupted
netsh winsock reset
```

### 3. Pre-start the daemon

In tests and production, ensure the daemon is started *before* attempting to connect. The daemon binds the port → `connect()` succeeds immediately.

## Affected Tests

E2E tests that rely on `is_alive()` returning `false` (fast-fail) may block or timeout on Windows machines with aggressive firewall/WFP filtering:

| Test | Status | Workaround |
|------|--------|------------|
| `test_ensure_daemon_daemon_dead_spawn_fails` | ✅ Unit test (no TCP) | Uses mock, bypasses `TcpStream` |
| `test_ensure_daemon_spawns_daemon` (e2e) | ❌ Removed | Cannot test "daemon dead → spawn" via e2e on Windows; use unit tests with mocks instead |

## References

- [Microsoft: ConnectEx documentation](https://learn.microsoft.com/en-us/windows/win32/api/mswsock/nc-mswsock-lpfn_connectex)
- [WFP: Windows Filtering Platform](https://learn.microsoft.com/en-us/windows/win32/fwp/windows-filtering-platform-start-page)
