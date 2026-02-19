# Deep Security Review - Oxicrab Codebase

**Date:** February 19, 2026  
**Reviewer:** Security Analysis  
**Scope:** Full codebase architecture and implementation

## Executive Summary

Oxicrab demonstrates **strong security fundamentals** with multiple defense-in-depth layers. The codebase shows thoughtful attention to common attack vectors including SSRF, credential leakage, injection attacks, and privilege escalation. However, several areas require attention to harden against advanced threats.

**Overall Security Posture:** ⚠️ **Good with room for improvement**

### Key Strengths
- Comprehensive SSRF protection with DNS pinning
- Multi-layer credential management (env → helper → keyring → config)
- Leak detection for secrets in outbound messages
- Strong input validation and sanitization
- Subprocess environment scrubbing
- Rate limiting and circuit breakers

### Critical Issues
1. **MCP Server Trust Model** - MCP servers run with user privileges; no sandboxing
2. **File Path Canonicalization Race** - TOCTOU vulnerabilities in file operations
3. **Error Message Information Disclosure** - Some errors leak internal paths/details
4. **Pairing Code Entropy** - 8-character codes may be brute-forceable

### High Priority Issues
5. **Shell Command Injection** - Regex-based filtering may have bypasses
6. **Config File Race Conditions** - Atomic writes but no locking
7. **MCP Environment Variable Injection** - Secrets passed via env vars despite warnings

---

## 1. Credential Management

### ✅ Strengths

**Multi-tier credential resolution:**
- Environment variables (`OXICRAB_*`) take precedence
- Credential helper support (1Password, Bitwarden, custom scripts)
- OS keyring integration (optional, `keyring-store` feature)
- Config file as fallback

**Implementation:** `src/config/credentials/mod.rs`

**Security Features:**
- Credentials never logged in debug output (redacted via `redact_debug!` macro)
- Empty credentials skipped during resolution
- Source detection for audit trail

### ⚠️ Concerns

1. **Credential Helper Subprocess Security**
   - Location: `src/config/credentials/mod.rs:199-230`
   - Uses `scrubbed_sync_command()` which clears environment
   - **Issue:** 30-second timeout may be insufficient for slow credential helpers
   - **Recommendation:** Make timeout configurable, add progress indicators

2. **Keyring Error Handling**
   - Location: `src/config/credentials/mod.rs:274-306`
   - Errors are logged but don't fail credential resolution
   - **Issue:** Silent failures may mask security issues
   - **Recommendation:** Add explicit error handling modes (fail-fast vs. fallback)

3. **Credential Helper Input Validation**
   - Location: `src/config/credentials/mod.rs:199-230`
   - Helper command/args come from config.json (user-controlled)
   - **Issue:** No validation of helper executable paths
   - **Recommendation:** Validate helper paths, restrict to allowlisted directories

---

## 2. Subprocess Execution Security

### ✅ Strengths

**Environment Scrubbing:**
- Location: `src/utils/subprocess.rs`
- `scrubbed_command()` clears all env vars, then adds only allowlisted vars:
  - `PATH`, `HOME`, `USER`, `LANG`, `LC_ALL`, `TZ`, `TERM`, `RUST_LOG`, `TMPDIR`, `XDG_RUNTIME_DIR`
- Applied to all subprocesses: shell exec, MCP servers, ffmpeg, tmux

**Shell Command Validation:**
- Location: `src/agent/tools/shell.rs`
- Multi-layer protection:
  1. Allowlist check (if configured)
  2. Blocklist regex patterns (dangerous commands)
  3. Workspace path validation
  4. Timeout enforcement
  5. Output size limits (1 MB)

### ⚠️ Critical Issues

1. **Shell Command Injection via Regex Bypass**
   - Location: `src/agent/tools/shell.rs:120-167`
   - Regex patterns may be bypassed with:
     - Unicode normalization issues
     - Command substitution via environment variables
     - Nested quoting: `sh -c 'sh -c "rm -rf /"'`
   - **Recommendation:** 
     - Use AST parsing instead of regex (e.g., `shlex` crate)
     - Whitelist-only approach (no regex blocklist)
     - Disable command substitution entirely

2. **MCP Server Privilege Escalation**
   - Location: `src/agent/tools/mcp/mod.rs:61-105`
   - MCP servers run with full user privileges
   - No sandboxing (no seccomp, no namespaces, no capabilities dropping)
   - **Issue:** Malicious MCP server can access all user files, network, etc.
   - **Recommendation:**
     - Implement sandboxing via `bubblewrap` or `firejail`
     - Drop capabilities (CAP_SYS_ADMIN, etc.)
     - Use seccomp filters
     - Consider running in containers

3. **Path Canonicalization Race (TOCTOU)**
   - Location: `src/agent/tools/filesystem/mod.rs:12-39`
   - `canonicalize()` resolves symlinks, but between check and use, symlink may change
   - **Issue:** Attacker could swap symlink after validation
   - **Recommendation:**
     - Use `openat()` with `O_NOFOLLOW` and `O_PATH`
     - Or use `readlinkat()` + `openat()` pattern
     - Consider `fchdir()` to pin directory

---

## 3. SSRF Protection

### ✅ Excellent Implementation

**DNS Rebinding Prevention:**
- Location: `src/utils/url_security/mod.rs`
- Validates URL → resolves DNS → pins IP addresses → builds reqwest client with `.resolve()`
- Prevents TOCTOU DNS rebinding attacks

**IP Address Blocking:**
- Blocks private IPs (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
- Blocks loopback (127.0.0.0/8, ::1)
- Blocks link-local (169.254.0.0/16, fe80::/10)
- Blocks multicast (224.0.0.0/4, ff00::/8)
- Blocks IPv4-mapped IPv6 addresses
- Blocks NAT64 well-known prefix (64:ff9b::/96)
- Blocks 6to4 tunneling (2002::/16)

**Redirect Disabling:**
- Location: `src/agent/tools/http/mod.rs:34`, `src/agent/tools/web/mod.rs:417`
- `redirect(reqwest::redirect::Policy::none())` prevents redirect-based SSRF bypass

### ⚠️ Minor Issues

1. **IPv6 Link-Local Detection**
   - Location: `src/utils/url_security/mod.rs:92`
   - Uses bitmask `segments[0] & 0xffc0 == 0xfe80`
   - **Issue:** Should also check for `fe80::/10` with proper prefix matching
   - **Status:** Actually correct — `0xffc0` mask checks first 10 bits

2. **DNS Resolution Timeout**
   - Location: `src/utils/url_security/mod.rs:47`
   - Uses `tokio::net::lookup_host()` with no explicit timeout
   - **Issue:** Slow DNS could cause DoS
   - **Recommendation:** Add DNS resolution timeout (5-10 seconds)

---

## 4. Secret Leak Detection

### ✅ Strong Implementation

**Multi-Encoding Detection:**
- Location: `src/safety/leak_detector.rs`
- Detects secrets in:
  - Plaintext (regex patterns)
  - Base64 encoding (standard + URL-safe)
  - Hex encoding
- Scans both user messages and LLM responses

**Known Secret Matching:**
- Registers actual config secrets for exact-match detection
- Creates regex patterns for raw, base64, and hex encodings
- Minimum 10-character threshold to avoid false positives

**Integration:**
- Location: `src/bus/queue.rs:116-128`
- Scans all outbound messages before sending
- Automatically redacts detected secrets

### ⚠️ Limitations

1. **Pattern Coverage**
   - Only covers: Anthropic, OpenAI, Slack, GitHub, AWS, Groq, Telegram, Discord
   - Missing: Twilio tokens, custom API keys, JWT tokens
   - **Recommendation:** Add more patterns, make configurable

2. **False Positives**
   - Base64 regex `[A-Za-z0-9+/]{20,500}` may match legitimate content
   - **Status:** Acceptable trade-off (better to redact false positives than leak secrets)

3. **Encoding Evasion**
   - Attacker could use:
     - ROT13, Caesar cipher
     - Custom encoding schemes
     - Steganography in images
   - **Status:** Inherent limitation of pattern-based detection

---

## 5. Input Validation & Sanitization

### ✅ Strengths

**Tool Parameter Validation:**
- Location: `src/agent/loop/mod.rs:74-128`
- Validates required fields, types (string, number, boolean, array, object)
- Returns clear error messages

**File Path Validation:**
- Location: `src/agent/tools/filesystem/mod.rs:12-39`
- Canonicalizes paths, checks against allowed roots
- Lexical normalization for non-existent paths (prevents `..` traversal)

**Shell Command Validation:**
- Location: `src/agent/tools/shell.rs:120-167`
- Multiple layers: allowlist, blocklist, workspace checks
- Normalizes line continuations (`\\\n` → space)

**Prompt Injection Detection:**
- Location: `src/safety/prompt_guard.rs`
- Detects role switching, instruction override, secret extraction, jailbreak patterns
- Unicode normalization to prevent zero-width character evasion

### ⚠️ Issues

1. **Unicode Normalization Bypass**
   - Location: `src/safety/prompt_guard.rs:126-152`
   - Strips zero-width chars, but attacker could use:
     - Homoglyphs (Cyrillic 'а' vs Latin 'a')
     - Combining characters
     - Right-to-left overrides
   - **Recommendation:** Use Unicode normalization (NFKC) instead of manual filtering

2. **File Path Traversal via Symlinks**
   - Location: `src/agent/tools/filesystem/mod.rs:17-19`
   - `canonicalize()` resolves symlinks, but race condition exists
   - **Issue:** See TOCTOU issue in Section 2

3. **HTTP Body Size Limits**
   - Location: `src/utils/http.rs:25-48`
   - Default 10 MB limit, but Content-Length header can be spoofed
   - **Status:** Actually safe — checks Content-Length first, then streams with counter

---

## 6. Authentication & Authorization

### ✅ Strengths

**DM Access Control:**
- Location: `src/channels/utils.rs:110-142`
- Three policies: `open`, `allowlist`, `pairing`
- Empty `allowFrom` arrays deny all (secure default)

**Pairing System:**
- Location: `src/pairing/mod.rs`
- 8-character codes, 15-minute TTL
- Per-client brute-force lockout (10 attempts per 5 minutes)
- Bounded lockout map (1000 clients max)

**Webhook Signature Validation:**
- Location: `src/channels/twilio.rs:58-81`
- Constant-time comparison via `subtle::ConstantTimeEq`
- HMAC-SHA1 signature validation

### ⚠️ Critical Issues

1. **Pairing Code Entropy**
   - Location: `src/pairing/mod.rs:9-10`
   - 8 characters from 32-character alphabet = 32^8 = ~1 trillion combinations
   - **Issue:** With 10 attempts per 5 minutes, attacker needs ~15.8 million years
   - **Status:** Actually secure for intended use case
   - **Concern:** If attacker can observe many codes, entropy decreases
   - **Recommendation:** Consider 10-character codes for high-security deployments

2. **Pairing Store File Permissions**
   - Location: `src/pairing/mod.rs:50-157`
   - Files stored at `~/.oxicrab/pairing/*-allowlist.json`
   - **Issue:** No explicit permission setting (relies on umask)
   - **Recommendation:** Explicitly set `0o600` permissions on pairing files

3. **DM Policy Default**
   - Location: `src/channels/utils.rs:116`
   - Default is `allowlist` (silent deny)
   - **Issue:** Users may not realize pairing is required
   - **Status:** Documented, but could be clearer

---

## 7. Rate Limiting & DoS Protection

### ✅ Strengths

**Message Rate Limiting:**
- Location: `src/bus/queue.rs:75-114`
- Per-sender rate limiting (default: 30 messages per 60 seconds)
- Bounded timestamp map (prunes at 1000 entries)

**Cost Guard:**
- Location: `src/agent/cost_guard.rs`
- Daily budget enforcement
- Hourly action rate limiting
- AtomicBool fast-path for already-exceeded budgets

**Circuit Breaker:**
- Location: `src/providers/circuit_breaker.rs`
- Three states: Closed, Open, HalfOpen
- Transient error detection (429, 5xx, timeout)
- Non-transient errors don't trip breaker (auth, invalid key)

### ⚠️ Issues

1. **Rate Limit Bypass via Multiple Senders**
   - Location: `src/bus/queue.rs:77`
   - Key is `format!("{}:{}", channel, sender_id)`
   - **Issue:** Attacker with multiple accounts can bypass limits
   - **Status:** Inherent limitation — consider IP-based limiting for webhooks

2. **Timestamp Map Memory Growth**
   - Location: `src/bus/queue.rs:97-101`
   - Prunes at 1000 entries, but only removes inactive senders
   - **Issue:** Active attackers could keep map large
   - **Status:** Acceptable — 1000 entries is bounded

---

## 8. Error Handling & Information Disclosure

### ⚠️ Issues

1. **Error Messages Leak Paths**
   - Location: `src/agent/tools/filesystem/mod.rs:154-170`
   - Errors include full file paths: `"Error: File not found: {}"`
   - **Issue:** May reveal directory structure
   - **Recommendation:** Sanitize paths in error messages (show basename only)

2. **Provider Error Messages**
   - Location: `src/providers/errors.rs:15-61`
   - Some errors include model names, API error details
   - **Status:** Generally safe — no secrets leaked

3. **Stack Traces in Production**
   - Location: Various `anyhow::bail!()` calls
   - **Issue:** Stack traces may leak internal details
   - **Recommendation:** Use structured error types, avoid `anyhow::Error` in user-facing code

---

## 9. File System Security

### ✅ Strengths

**Atomic Writes:**
- Location: `src/utils/mod.rs:43-55`
- Uses `tempfile::NamedTempFile` + `persist()` for atomic writes
- Prevents corruption on crash

**Config File Permissions:**
- Location: `src/config/loader/mod.rs:138-142`
- Sets `0o600` permissions on Unix
- Warns if config file is world-readable

**File Size Limits:**
- Read: 10 MB (`src/agent/tools/filesystem/mod.rs:10`)
- HTTP bodies: 10 MB (`src/utils/http.rs:6`)
- Shell output: 1 MB (`src/agent/tools/shell.rs:13`)
- Media files: 20 MB (`src/utils/media.rs:4`)

### ⚠️ Issues

1. **Config File Race Conditions**
   - Location: `src/config/loader/mod.rs:121-145`
   - Atomic writes, but no file locking
   - **Issue:** Concurrent writes could corrupt config
   - **Recommendation:** Use `flock()` or `fcntl()` advisory locks

2. **Backup File Permissions**
   - Location: `src/agent/tools/filesystem/mod.rs:46-95`
   - Backup files inherit permissions from parent directory
   - **Issue:** May be world-readable if directory permissions are loose
   - **Recommendation:** Explicitly set `0o600` on backup files

---

## 10. Network Security

### ✅ Strengths

**HTTP Client Timeouts:**
- Connect: 10 seconds
- Overall: 30 seconds
- Applied consistently across all HTTP clients

**TLS Configuration:**
- Uses `rustls` (pure Rust, no OpenSSL)
- No custom certificate validation (uses system trust store)

**User-Agent Headers:**
- Location: `src/agent/tools/http/mod.rs:31`
- Sets `oxicrab/{version}` user-agent

### ⚠️ Issues

1. **No Certificate Pinning**
   - **Issue:** Relies on system trust store
   - **Status:** Generally acceptable, but consider pinning for critical APIs

2. **No Request ID Tracking**
   - **Issue:** Difficult to correlate requests in logs
   - **Recommendation:** Add `X-Request-ID` header to all requests

---

## 11. MCP Server Security

### ⚠️ Critical Issues

1. **No Sandboxing**
   - Location: `src/agent/tools/mcp/mod.rs:61-105`
   - MCP servers run with full user privileges
   - **Risk:** Malicious server can:
     - Read/write all user files
     - Access network
     - Escalate privileges
   - **Recommendation:** Implement sandboxing (see Section 2)

2. **Environment Variable Injection**
   - Location: `src/agent/tools/mcp/mod.rs:70-88`
   - Warns about secrets in env vars, but still passes them
   - **Issue:** Secrets visible to MCP server process
   - **Recommendation:** Use credential helper or file-based secrets

3. **Trust Level Not Enforced**
   - Location: `src/agent/tools/mcp/mod.rs:109`
   - Returns `(trust_level, tool)` tuples, but trust level may not be checked
   - **Issue:** Untrusted tools may be executed
   - **Recommendation:** Verify trust level is checked before tool execution

---

## 12. Memory Safety

### ✅ Strengths

**Rust Memory Safety:**
- No unsafe blocks in security-critical paths (verified via grep)
- Bounded allocations (size limits on all inputs)

**Output Truncation:**
- Shell output: 1 MB
- HTTP bodies: 10 MB
- File reads: 10 MB

### ⚠️ Minor Issues

1. **Regex DoS Potential**
   - Location: `src/safety/leak_detector.rs:96-97`
   - Base64 regex `[A-Za-z0-9+/]{20,500}` could be slow on large inputs
   - **Status:** Bounded by message size limits, acceptable

---

## Recommendations Summary

### Critical (Fix Immediately)
1. **MCP Server Sandboxing** - Implement sandboxing for MCP servers
2. **Shell Command Injection** - Replace regex with AST parsing
3. **TOCTOU File Operations** - Use `openat()` pattern for file operations

### High Priority (Fix Soon)
4. **Pairing Code Entropy** - Consider 10-character codes for high-security deployments
5. **Config File Locking** - Add advisory file locks for concurrent writes
6. **Error Message Sanitization** - Remove full paths from error messages

### Medium Priority (Fix When Convenient)
7. **Credential Helper Validation** - Validate helper executable paths
8. **DNS Resolution Timeout** - Add explicit timeout for DNS lookups
9. **Backup File Permissions** - Explicitly set permissions on backup files
10. **Unicode Normalization** - Use NFKC normalization instead of manual filtering

### Low Priority (Nice to Have)
11. **Certificate Pinning** - Consider pinning for critical APIs
12. **Request ID Tracking** - Add request IDs for better logging
13. **Pattern Coverage** - Expand secret leak detection patterns

---

## Conclusion

Oxicrab demonstrates **strong security fundamentals** with defense-in-depth across multiple layers. The SSRF protection, credential management, and leak detection are particularly well-implemented. However, the MCP server trust model and shell command validation need hardening to resist advanced attackers.

**Overall Assessment:** The codebase is **production-ready for trusted environments** but requires the critical fixes listed above before deployment in untrusted or high-security environments.

---

## Appendix: Security Checklist

- [x] Credential management (multi-tier)
- [x] SSRF protection (DNS pinning, IP blocking)
- [x] Secret leak detection (multi-encoding)
- [x] Input validation (tool parameters, file paths)
- [x] Rate limiting (messages, costs)
- [x] Subprocess environment scrubbing
- [x] Webhook signature validation
- [x] File size limits
- [x] Atomic file writes
- [ ] MCP server sandboxing (CRITICAL)
- [ ] Shell command AST parsing (CRITICAL)
- [ ] TOCTOU file operation fixes (CRITICAL)
- [ ] Config file locking (HIGH)
- [ ] Error message sanitization (HIGH)
