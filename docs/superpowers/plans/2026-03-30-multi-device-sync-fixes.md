# Multi-Device Sync Fixes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two bugs that block multi-device sync: `generate_delta` failing for self-identity peers, and the relay not auto-registering new device keys on login.

**Architecture:** Fix 1 adds a self-identity check in `generate_delta` before the contact manager lookup. Fix 2 modifies the relay server's `LoginHandler` to conditionally include a PoP challenge for unknown device keys, and updates the Rust client + Tauri command to auto-verify when a challenge is returned.

**Tech Stack:** Rust (krillnotes-core), PHP (krillnotes-relay), Tauri v2

**Spec:** `docs/superpowers/specs/2026-03-30-multi-device-sync-fixes-design.md`

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `krillnotes-core/src/core/swarm/sync.rs` | Self-identity bypass in `generate_delta` |
| Modify | `krillnotes-core/src/core/sync/relay/client.rs` | Add optional `challenge` to `SessionResponse` |
| Modify | `krillnotes-desktop/src-tauri/src/commands/relay_accounts.rs` | Auto-verify device in `login_relay_account` |
| Modify | `~/Source/krillnotes-relay/src/Handler/Auth/LoginHandler.php` | Conditional device registration on login |
| Modify | `~/Source/krillnotes-relay/config/container.php` | Inject new deps into `LoginHandler` |
| Modify | `~/Source/krillnotes-relay/tests/Integration/Auth/LoginFlowTest.php` | Tests for new login+device flow |

---

## Task 1: `generate_delta` self-identity bypass

**Files:**
- Modify: `krillnotes-core/src/core/swarm/sync.rs:96-112`
- Test: same file, `#[cfg(test)]` module (line 301+)

- [ ] **Step 1: Write the failing test**

Add this test to the `mod tests` block at the bottom of `krillnotes-core/src/core/swarm/sync.rs`, after the existing `test_generate_delta_no_watermark_includes_all_ops` test (after line 427):

```rust
    /// generate_delta succeeds for a self-identity peer (multi-device sync).
    /// No contact entry exists for the peer because it's the same identity.
    #[test]
    fn test_generate_delta_self_identity_peer() {
        let alice_key = make_key();
        let alice_pubkey_b64 = b64(&alice_key);

        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut ws = crate::core::workspace::Workspace::create(
            temp.path(),
            "",
            "alice-id",
            SigningKey::from_bytes(&alice_key.to_bytes()),
            test_gate(),
            None,
        )
        .unwrap();

        // Register a self-identity peer (same pubkey, different device).
        let snap_op = ws.get_latest_operation_id().unwrap().unwrap_or_default();
        ws.upsert_sync_peer(
            "device-other:identity:alice-id",
            &alice_pubkey_b64,
            Some(&snap_op),
            None,
        )
        .unwrap();

        // Empty contact manager — no contact for self.
        let cm_dir = tempfile::tempdir().unwrap();
        let cm = crate::core::contact::ContactManager::for_identity(
            cm_dir.path().to_path_buf(),
            [3u8; 32],
        )
        .unwrap();

        // generate_delta must succeed without a contact entry.
        let bundle = super::generate_delta(
            &mut ws,
            "device-other:identity:alice-id",
            "TestWorkspace",
            &alice_key,
            "Alice",
            &cm,
        )
        .unwrap();

        // Parse with alice's own key (same identity on both devices).
        let parsed =
            crate::core::swarm::delta::parse_delta_bundle(&bundle.bundle_bytes, &alice_key)
                .unwrap();
        assert_eq!(parsed.workspace_id, ws.workspace_id());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p krillnotes-core test_generate_delta_self_identity_peer -- --nocapture 2>&1 | tail -20`

Expected: FAIL with `Swarm("no contact for peer identity ...")`

- [ ] **Step 3: Implement the self-identity bypass**

In `krillnotes-core/src/core/swarm/sync.rs`, replace lines 96–112 (the contact lookup + key parsing block):

```rust
    // 4. Resolve peer's public key from contacts.
    let contact = contact_manager
        .find_by_public_key(&peer.peer_identity_id)?
        .ok_or_else(|| {
            KrillnotesError::Swarm(format!(
                "no contact for peer identity {}",
                peer.peer_identity_id
            ))
        })?;
    let recipient_key_bytes = BASE64
        .decode(&contact.public_key)
        .map_err(|e| KrillnotesError::Swarm(format!("bad contact public key: {e}")))?;
    let recipient_key_arr: [u8; 32] = recipient_key_bytes.try_into().map_err(|_| {
        KrillnotesError::Swarm("contact public key wrong length".to_string())
    })?;
    let recipient_vk = VerifyingKey::from_bytes(&recipient_key_arr)
        .map_err(|e| KrillnotesError::Swarm(format!("invalid recipient key: {e}")))?;
```

With:

```rust
    // 4. Resolve peer's public key.
    //    For self-identity peers (multi-device sync) the signing key's verifying
    //    key IS the recipient key — skip the contact manager lookup.
    let sender_vk = signing_key.verifying_key();
    let sender_pubkey_b64 = BASE64.encode(sender_vk.as_bytes());

    let recipient_vk = if peer.peer_identity_id == sender_pubkey_b64 {
        sender_vk
    } else {
        let contact = contact_manager
            .find_by_public_key(&peer.peer_identity_id)?
            .ok_or_else(|| {
                KrillnotesError::Swarm(format!(
                    "no contact for peer identity {}",
                    peer.peer_identity_id
                ))
            })?;
        let recipient_key_bytes = BASE64
            .decode(&contact.public_key)
            .map_err(|e| KrillnotesError::Swarm(format!("bad contact public key: {e}")))?;
        let recipient_key_arr: [u8; 32] = recipient_key_bytes.try_into().map_err(|_| {
            KrillnotesError::Swarm("contact public key wrong length".to_string())
        })?;
        VerifyingKey::from_bytes(&recipient_key_arr)
            .map_err(|e| KrillnotesError::Swarm(format!("invalid recipient key: {e}")))?
    };
```

- [ ] **Step 4: Run all sync tests to verify they pass**

Run: `cargo test -p krillnotes-core swarm::sync::tests -- --nocapture 2>&1 | tail -20`

Expected: All tests pass, including the new `test_generate_delta_self_identity_peer`.

- [ ] **Step 5: Commit**

```bash
git add krillnotes-core/src/core/swarm/sync.rs
git commit -m "fix: generate_delta bypass contact lookup for self-identity peers

Multi-device sync registers the same identity as a peer. The contact
manager has no entry for 'self', so generate_delta failed with
'no contact for peer identity'. Now detects when the peer's identity
matches the sender's signing key and uses the verifying key directly."
```

---

## Task 2: Relay server — conditional device registration on login

**Files:**
- Modify: `~/Source/krillnotes-relay/src/Handler/Auth/LoginHandler.php`
- Modify: `~/Source/krillnotes-relay/config/container.php:87-94`
- Test: `~/Source/krillnotes-relay/tests/Integration/Auth/LoginFlowTest.php`

- [ ] **Step 1: Write the failing test — login with unknown device key returns challenge**

Add this test to `~/Source/krillnotes-relay/tests/Integration/Auth/LoginFlowTest.php`, after the existing `test_login_with_nonexistent_email_returns_401` test (after line 127):

```php
    public function test_login_with_unknown_device_key_returns_challenge(): void
    {
        $this->registerAccount('dan@example.com', 'securepass');

        // Generate a SECOND Ed25519 keypair (simulates a new device)
        $edKp2 = sodium_crypto_sign_keypair();
        $edPk2 = sodium_crypto_sign_publickey($edKp2);
        $edPk2Hex = bin2hex($edPk2);

        $request = (new ServerRequestFactory())->createServerRequest('POST', '/auth/login')
            ->withParsedBody([
                'email' => 'dan@example.com',
                'password' => 'securepass',
                'device_public_key' => $edPk2Hex,
            ]);
        $response = ($this->loginHandler)($request);

        $this->assertSame(200, $response->getStatusCode());
        $data = json_decode((string) $response->getBody(), true)['data'];
        $this->assertArrayHasKey('session_token', $data);
        $this->assertNotEmpty($data['session_token']);
        $this->assertArrayHasKey('challenge', $data);
        $this->assertArrayHasKey('encrypted_nonce', $data['challenge']);
        $this->assertArrayHasKey('server_public_key', $data['challenge']);
    }
```

- [ ] **Step 2: Write the failing test — login with known+verified device key returns no challenge**

Add this test right after the previous one:

```php
    public function test_login_with_verified_device_key_returns_no_challenge(): void
    {
        $this->registerAccount('eve@example.com', 'mypassword');

        // Log in with the SAME device key that was registered (already verified).
        // We need the device key from registration, so refactor registerAccount
        // to also return it.
        $edKp = sodium_crypto_sign_keypair();
        $edPk = sodium_crypto_sign_publickey($edKp);
        $edPkHex = bin2hex($edPk);

        // Register with this specific key
        $edSk = sodium_crypto_sign_secretkey($edKp);
        $registerHandler = new \Relay\Handler\Auth\RegisterHandler(
            $this->accounts,
            new DeviceKeyRepository($this->pdo),
            new ChallengeRepository($this->pdo),
            $this->auth,
            new CryptoService(),
            $this->settings,
        );
        $regRequest = (new ServerRequestFactory())->createServerRequest('POST', '/auth/register')
            ->withParsedBody([
                'email' => 'eve@example.com',
                'password' => 'mypassword',
                'identity_uuid' => 'id-uuid-eve',
                'device_public_key' => $edPkHex,
            ]);
        $regResponse = $registerHandler($regRequest);
        $regData = json_decode((string) $regResponse->getBody(), true)['data'];

        // Complete PoP verification
        $clientX25519Sk = sodium_crypto_sign_ed25519_sk_to_curve25519($edSk);
        $serverX25519Pk = hex2bin($regData['challenge']['server_public_key']);
        $blob = hex2bin($regData['challenge']['encrypted_nonce']);
        $boxNonce = substr($blob, 0, SODIUM_CRYPTO_BOX_NONCEBYTES);
        $ciphertext = substr($blob, SODIUM_CRYPTO_BOX_NONCEBYTES);
        $decryptKp = sodium_crypto_box_keypair_from_secretkey_and_publickey($clientX25519Sk, $serverX25519Pk);
        $plaintext = sodium_crypto_box_open($ciphertext, $boxNonce, $decryptKp);

        $verifyHandler = new \Relay\Handler\Auth\RegisterVerifyHandler(
            new ChallengeRepository($this->pdo),
            new DeviceKeyRepository($this->pdo),
            $this->sessions,
            new CryptoService(),
            $this->settings,
        );
        $verifyRequest = (new ServerRequestFactory())->createServerRequest('POST', '/auth/register/verify')
            ->withParsedBody(['device_public_key' => $edPkHex, 'nonce' => bin2hex($plaintext)]);
        $verifyHandler($verifyRequest);

        // Now login with the same verified key — should NOT include a challenge.
        $request = (new ServerRequestFactory())->createServerRequest('POST', '/auth/login')
            ->withParsedBody([
                'email' => 'eve@example.com',
                'password' => 'mypassword',
                'device_public_key' => $edPkHex,
            ]);
        $response = ($this->loginHandler)($request);

        $this->assertSame(200, $response->getStatusCode());
        $data = json_decode((string) $response->getBody(), true)['data'];
        $this->assertArrayHasKey('session_token', $data);
        $this->assertArrayNotHasKey('challenge', $data);
    }
```

- [ ] **Step 3: Write the failing test — login without device key returns no challenge (backward compat)**

Add this test right after the previous one:

```php
    public function test_login_without_device_key_returns_no_challenge(): void
    {
        $this->registerAccount('frank@example.com', 'frankpass');

        $request = (new ServerRequestFactory())->createServerRequest('POST', '/auth/login')
            ->withParsedBody(['email' => 'frank@example.com', 'password' => 'frankpass']);
        $response = ($this->loginHandler)($request);

        $this->assertSame(200, $response->getStatusCode());
        $data = json_decode((string) $response->getBody(), true)['data'];
        $this->assertArrayHasKey('session_token', $data);
        $this->assertArrayNotHasKey('challenge', $data);
    }
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cd ~/Source/krillnotes-relay && php vendor/bin/phpunit tests/Integration/Auth/LoginFlowTest.php 2>&1 | tail -20`

Expected: The two new device-key tests fail (no `challenge` key in response). The backward compat test should already pass.

- [ ] **Step 5: Update the DI container to inject new dependencies into LoginHandler**

In `~/Source/krillnotes-relay/config/container.php`, replace the LoginHandler entry (lines 87-94):

```php
    \Relay\Handler\Auth\LoginHandler::class => function ($c) {
        return new \Relay\Handler\Auth\LoginHandler(
            $c->get(\Relay\Repository\AccountRepository::class),
            $c->get(\Relay\Repository\SessionRepository::class),
            $c->get(\Relay\Service\AuthService::class),
            $c->get('settings'),
        );
    },
```

With:

```php
    \Relay\Handler\Auth\LoginHandler::class => function ($c) {
        return new \Relay\Handler\Auth\LoginHandler(
            $c->get(\Relay\Repository\AccountRepository::class),
            $c->get(\Relay\Repository\SessionRepository::class),
            $c->get(\Relay\Service\AuthService::class),
            $c->get(\Relay\Repository\DeviceKeyRepository::class),
            $c->get(\Relay\Repository\ChallengeRepository::class),
            $c->get(\Relay\Service\CryptoService::class),
            $c->get('settings'),
        );
    },
```

- [ ] **Step 6: Implement conditional device registration in LoginHandler**

Replace the entire contents of `~/Source/krillnotes-relay/src/Handler/Auth/LoginHandler.php` with:

```php
<?php

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

declare(strict_types=1);
namespace Relay\Handler\Auth;
use Psr\Http\Message\ResponseInterface;
use Psr\Http\Message\ServerRequestInterface;
use Relay\Repository\AccountRepository;
use Relay\Repository\ChallengeRepository;
use Relay\Repository\DeviceKeyRepository;
use Relay\Repository\SessionRepository;
use Relay\Service\AuthService;
use Relay\Service\CryptoService;
use Slim\Psr7\Response;
final class LoginHandler
{
    public function __construct(
        private readonly AccountRepository $accounts,
        private readonly SessionRepository $sessions,
        private readonly AuthService $auth,
        private readonly DeviceKeyRepository $deviceKeys,
        private readonly ChallengeRepository $challenges,
        private readonly CryptoService $crypto,
        private readonly array $settings,
    ) {}
    public function __invoke(ServerRequestInterface $request): ResponseInterface
    {
        $body = $request->getParsedBody();
        $email = $body['email'] ?? '';
        $password = $body['password'] ?? '';
        if (!$email || !$password) {
            return $this->json(400, ['error' => ['code' => 'MISSING_FIELDS', 'message' => 'email and password are required']]);
        }
        $account = $this->accounts->findByEmail($email);
        if ($account === null || !$this->auth->verifyPassword($password, $account['password_hash'])) {
            return $this->json(401, ['error' => ['code' => 'INVALID_CREDENTIALS', 'message' => 'Invalid email or password']]);
        }
        if ($account['flagged_for_deletion'] !== null) {
            return $this->json(403, ['error' => ['code' => 'ACCOUNT_DELETED', 'message' => 'Account is flagged for deletion']]);
        }
        $token = $this->sessions->create($account['account_id'], $this->settings['auth']['session_lifetime_seconds']);

        $responseData = ['session_token' => $token];

        // Conditional device registration: if client sent a device_public_key,
        // check whether it's already registered and verified.
        $devicePublicKey = $body['device_public_key'] ?? '';
        if ($devicePublicKey !== '' && ctype_xdigit($devicePublicKey) && strlen($devicePublicKey) === 64) {
            $existing = $this->deviceKeys->findByKey($devicePublicKey);
            if ($existing === null) {
                // Unknown key — insert as unverified + issue PoP challenge.
                $this->deviceKeys->add($account['account_id'], $devicePublicKey);
                $challenge = $this->crypto->createChallenge($devicePublicKey);
                $this->challenges->create(
                    $account['account_id'],
                    $devicePublicKey,
                    $challenge['plaintext_nonce'],
                    $challenge['server_public_key'],
                    'device_add',
                    $this->settings['auth']['challenge_lifetime_seconds'],
                );
                $responseData['challenge'] = [
                    'encrypted_nonce' => $challenge['encrypted_nonce'],
                    'server_public_key' => $challenge['server_public_key'],
                ];
            } elseif (!(bool) $existing['verified']) {
                // Known but unverified — issue fresh PoP challenge.
                $challenge = $this->crypto->createChallenge($devicePublicKey);
                $this->challenges->create(
                    $account['account_id'],
                    $devicePublicKey,
                    $challenge['plaintext_nonce'],
                    $challenge['server_public_key'],
                    'device_add',
                    $this->settings['auth']['challenge_lifetime_seconds'],
                );
                $responseData['challenge'] = [
                    'encrypted_nonce' => $challenge['encrypted_nonce'],
                    'server_public_key' => $challenge['server_public_key'],
                ];
            }
            // else: verified — no challenge needed
        }

        return $this->json(200, ['data' => $responseData]);
    }
    private function json(int $status, array $data): ResponseInterface
    {
        $response = new Response($status);
        $response->getBody()->write(json_encode($data));
        return $response->withHeader('Content-Type', 'application/json');
    }
}
```

- [ ] **Step 7: Update the test setUp to inject new deps into LoginHandler**

In `~/Source/krillnotes-relay/tests/Integration/Auth/LoginFlowTest.php`, update the `setUp()` method. Replace lines 39-43:

```php
        $this->loginHandler = new LoginHandler(
            $this->accounts,
            $this->sessions,
            $this->auth,
            $this->settings,
        );
```

With:

```php
        $this->loginHandler = new LoginHandler(
            $this->accounts,
            $this->sessions,
            $this->auth,
            new DeviceKeyRepository($this->pdo),
            new ChallengeRepository($this->pdo),
            new CryptoService(),
            $this->settings,
        );
```

- [ ] **Step 8: Run all login tests to verify they pass**

Run: `cd ~/Source/krillnotes-relay && php vendor/bin/phpunit tests/Integration/Auth/LoginFlowTest.php 2>&1 | tail -30`

Expected: All tests pass — existing tests still work, new tests verify device challenge behavior.

- [ ] **Step 9: Commit (relay repo)**

```bash
cd ~/Source/krillnotes-relay
git add src/Handler/Auth/LoginHandler.php config/container.php tests/Integration/Auth/LoginFlowTest.php
git commit -m "feat: auto-register unknown device keys on login

When login includes a device_public_key that isn't registered or isn't
verified, the response now includes a PoP challenge inline. Already-
verified keys see no change (zero overhead beyond one DB lookup).
Backward compatible: omitting device_public_key works as before."
```

---

## Task 3: Rust client — optional challenge in SessionResponse

**Files:**
- Modify: `krillnotes-core/src/core/sync/relay/client.rs:37-40`

- [ ] **Step 1: Update `SessionResponse` to include optional challenge**

In `krillnotes-core/src/core/sync/relay/client.rs`, replace lines 37-40:

```rust
#[derive(Debug, Deserialize)]
pub struct SessionResponse {
    pub session_token: String,
}
```

With:

```rust
#[derive(Debug, Deserialize)]
pub struct SessionResponse {
    pub session_token: String,
    /// Present when the server detected an unknown or unverified device key
    /// during login. The client should decrypt the nonce and call `verify_device`.
    pub challenge: Option<RegisterChallenge>,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p krillnotes-core --features relay 2>&1 | tail -10`

Expected: Compiles successfully. The `challenge` field is `Option` so existing JSON without it will deserialize as `None`.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-core/src/core/sync/relay/client.rs
git commit -m "feat: SessionResponse includes optional PoP challenge

When the relay detects an unknown device key during login it now returns
a challenge inline. The Option field deserializes as None for servers
that don't send it (backward compatible)."
```

---

## Task 4: Tauri command — auto-verify device after login

**Files:**
- Modify: `krillnotes-desktop/src-tauri/src/commands/relay_accounts.rs:156-226`

- [ ] **Step 1: Update `login_relay_account` to handle the challenge**

In `krillnotes-desktop/src-tauri/src/commands/relay_accounts.rs`, replace the `login_relay_account` function (lines 156-226) with:

```rust
#[tauri::command]
pub async fn login_relay_account(
    state: State<'_, AppState>,
    identity_uuid: String,
    relay_url: String,
    email: String,
    password: String,
) -> Result<RelayAccountInfo, String> {
    log::debug!("login_relay_account(identity={identity_uuid}, relay_url={relay_url})");
    let uuid = Uuid::parse_str(&identity_uuid).map_err(|e| e.to_string())?;

    let (signing_key, device_public_key) = {
        let m = state.unlocked_identities.lock().map_err(|e| e.to_string())?;
        let id = m.get(&uuid)
            .ok_or("Identity is not unlocked — please unlock your identity first")?;
        let sk = id.signing_key.clone();
        let dpk = hex::encode(id.verifying_key.to_bytes());
        (sk, dpk)
    };

    let relay_url_clone = relay_url.clone();
    let email_clone = email.clone();
    let password_clone = password.clone();
    let dpk = device_public_key.clone();

    // RelayClient uses reqwest::blocking — must run in spawn_blocking.
    let session_token = tokio::task::spawn_blocking(move || {
        let client = RelayClient::new(&relay_url_clone);
        let session = client
            .login(&email_clone, &password_clone, &dpk)
            .map_err(|e| e.to_string())?;

        // If the relay returned a PoP challenge (unknown/unverified device),
        // decrypt and verify automatically.
        if let Some(challenge) = &session.challenge {
            log::info!(target: "krillnotes::relay", "login returned device challenge — auto-verifying");
            let nonce_bytes = decrypt_pop_challenge(
                &signing_key,
                &challenge.encrypted_nonce,
                &challenge.server_public_key,
            )
            .map_err(|e| e.to_string())?;
            let nonce_hex = hex::encode(&nonce_bytes);

            let authed_client = RelayClient::new(&client.base_url)
                .with_session_token(&session.session_token);
            authed_client
                .verify_device(&dpk, &nonce_hex)
                .map_err(|e| e.to_string())?;
            log::info!(target: "krillnotes::relay", "device verified successfully");
        }

        Ok::<_, String>(session.session_token)
    })
    .await
    .map_err(|e| {
        log::error!("login_relay_account spawn_blocking join failed: {e}");
        e.to_string()
    })??;

    let session_expires_at = Utc::now() + chrono::Duration::days(30);

    // Update existing account or create a new one.
    let managers = state.relay_account_managers.lock().expect("Mutex poisoned");
    let mgr = managers.get(&uuid).ok_or("Identity not unlocked")?;

    let account = if let Some(mut existing) = mgr.find_by_url(&relay_url).map_err(|e| e.to_string())? {
        // Update the existing account with the new session.
        existing.email = email;
        existing.password = password;
        existing.session_token = session_token;
        existing.session_expires_at = session_expires_at;
        existing.device_public_key = device_public_key;
        mgr.save_relay_account(&existing).map_err(|e| {
            log::error!("login_relay_account: save_relay_account failed: {e}");
            e.to_string()
        })?;
        existing
    } else {
        mgr.create_relay_account(
            &relay_url,
            &email,
            &password,
            &session_token,
            session_expires_at,
            &device_public_key,
        )
        .map_err(|e| {
            log::error!("login_relay_account: create_relay_account failed: {e}");
            e.to_string()
        })?
    };

    Ok(RelayAccountInfo::from_account(&account))
}
```

Key changes from the original:
- Captures `signing_key` alongside `device_public_key` (line 8-12, same pattern as `register_relay_account`)
- After `client.login()`, checks `session.challenge` (line 11 of the spawn_blocking block)
- If present: decrypts nonce with `decrypt_pop_challenge`, creates an authenticated client, calls `verify_device`
- Rest of the function (account storage) is unchanged

- [ ] **Step 2: Verify it compiles**

Run: `cd krillnotes-desktop && cargo check -p krillnotes-desktop 2>&1 | tail -10`

Expected: Compiles successfully.

- [ ] **Step 3: Commit**

```bash
git add krillnotes-desktop/src-tauri/src/commands/relay_accounts.rs
git commit -m "feat: auto-verify device key after relay login

When the relay returns a PoP challenge in the login response (unknown or
unverified device key), the login command now automatically decrypts the
nonce and verifies the device. Same pattern as register_relay_account."
```

---

## Task 5: Build verification

- [ ] **Step 1: Run full core test suite**

Run: `cargo test -p krillnotes-core 2>&1 | tail -20`

Expected: All tests pass.

- [ ] **Step 2: Run full desktop check**

Run: `cd krillnotes-desktop && cargo check -p krillnotes-desktop 2>&1 | tail -10`

Expected: No errors.

- [ ] **Step 3: Run TypeScript type check**

Run: `cd krillnotes-desktop && npx tsc --noEmit 2>&1 | tail -10`

Expected: No errors (no TS changes in this PR, but verify nothing is broken).

- [ ] **Step 4: Final commit if any fixups needed, then done**
