//! Authentication FFI exports: status, password management, and passkey operations.

use std::ffi::c_char;

use secrecy::ExposeSecret;

use crate::{
    helpers::{
        encode_error, encode_json, parse_ffi_request, record_call, record_error, trace_call,
        with_ffi_boundary,
    },
    state::BRIDGE,
    types::*,
};

/// Returns authentication status for the HTTP server.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_status() -> *mut c_char {
    record_call("moltis_auth_status");
    trace_call("moltis_auth_status");

    with_ffi_boundary(|| {
        let has_password = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.has_password())
        {
            Ok(value) => value,
            Err(error) => {
                record_error("moltis_auth_status", "AUTH_STATUS_FAILED");
                return encode_error("AUTH_STATUS_FAILED", &error.to_string());
            },
        };

        let has_passkeys = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.has_passkeys())
        {
            Ok(value) => value,
            Err(error) => {
                record_error("moltis_auth_status", "AUTH_STATUS_FAILED");
                return encode_error("AUTH_STATUS_FAILED", &error.to_string());
            },
        };

        encode_json(&AuthStatusResponse {
            auth_disabled: BRIDGE.credential_store.is_auth_disabled(),
            has_password,
            has_passkeys,
            setup_complete: BRIDGE.credential_store.is_setup_complete(),
        })
    })
}

/// Adds or changes the authentication password.
///
/// Accepts JSON:
/// - `{"new_password":"..."}` to set the first password.
/// - `{"current_password":"...","new_password":"..."}` to rotate.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_password_change(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_auth_password_change");
    trace_call("moltis_auth_password_change");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<AuthPasswordChangeRequest>(
            "moltis_auth_password_change",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        if request.new_password.expose_secret().len() < 12 {
            record_error("moltis_auth_password_change", "AUTH_PASSWORD_TOO_SHORT");
            return encode_error(
                "AUTH_PASSWORD_TOO_SHORT",
                "new password must be at least 12 characters",
            );
        }

        let has_password = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.has_password())
        {
            Ok(value) => value,
            Err(error) => {
                record_error("moltis_auth_password_change", "AUTH_STATUS_FAILED");
                return encode_error("AUTH_STATUS_FAILED", &error.to_string());
            },
        };

        let mut recovery_key: Option<String> = None;

        let new_password = request.new_password.expose_secret();

        if has_password {
            let current_password = request
                .current_password
                .as_ref()
                .map(|s| s.expose_secret().as_str())
                .unwrap_or("");
            if let Err(error) = BRIDGE.runtime.block_on(
                BRIDGE
                    .credential_store
                    .change_password(current_password, new_password),
            ) {
                let message = error.to_string();
                if message.contains("incorrect") {
                    record_error(
                        "moltis_auth_password_change",
                        "AUTH_INVALID_CURRENT_PASSWORD",
                    );
                    return encode_error("AUTH_INVALID_CURRENT_PASSWORD", &message);
                }
                record_error("moltis_auth_password_change", "AUTH_PASSWORD_CHANGE_FAILED");
                return encode_error("AUTH_PASSWORD_CHANGE_FAILED", &message);
            }

            if let Some(vault) = BRIDGE.credential_store.vault()
                && let Err(error) = BRIDGE
                    .runtime
                    .block_on(vault.change_password(current_password, new_password))
            {
                crate::callbacks::emit_log(
                    "WARN",
                    "bridge.auth",
                    &format!("Vault password rotation failed: {error}"),
                );
            }
        } else if let Err(error) = BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.add_password(new_password))
        {
            record_error("moltis_auth_password_change", "AUTH_PASSWORD_SET_FAILED");
            return encode_error("AUTH_PASSWORD_SET_FAILED", &error.to_string());
        } else if let Some(vault) = BRIDGE.credential_store.vault() {
            match BRIDGE.runtime.block_on(vault.initialize(new_password)) {
                Ok(key) => {
                    recovery_key = Some(key.phrase().to_owned());
                },
                Err(moltis_gateway::auth::moltis_vault::VaultError::AlreadyInitialized) => {
                    if let Err(error) = BRIDGE.runtime.block_on(vault.unseal(new_password)) {
                        crate::callbacks::emit_log(
                            "WARN",
                            "bridge.auth",
                            &format!("Vault unseal failed after password set: {error}"),
                        );
                    }
                },
                Err(error) => {
                    crate::callbacks::emit_log(
                        "WARN",
                        "bridge.auth",
                        &format!("Vault initialization failed after password set: {error}"),
                    );
                },
            }
        }

        encode_json(&AuthPasswordChangeResponse {
            ok: true,
            recovery_key,
        })
    })
}

/// Removes all authentication credentials (passwords, passkeys, sessions, API keys).
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_reset() -> *mut c_char {
    record_call("moltis_auth_reset");
    trace_call("moltis_auth_reset");

    with_ffi_boundary(
        || match BRIDGE.runtime.block_on(BRIDGE.credential_store.reset_all()) {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error("moltis_auth_reset", "AUTH_RESET_FAILED");
                encode_error("AUTH_RESET_FAILED", &error.to_string())
            },
        },
    )
}

/// Lists all registered passkeys.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_list_passkeys() -> *mut c_char {
    record_call("moltis_auth_list_passkeys");
    trace_call("moltis_auth_list_passkeys");

    with_ffi_boundary(|| {
        let passkeys = match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.list_passkeys())
        {
            Ok(entries) => entries,
            Err(error) => {
                record_error("moltis_auth_list_passkeys", "AUTH_PASSKEY_LIST_FAILED");
                return encode_error("AUTH_PASSKEY_LIST_FAILED", &error.to_string());
            },
        };

        encode_json(&AuthPasskeysResponse { passkeys })
    })
}

/// Removes a passkey by database ID.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_remove_passkey(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_auth_remove_passkey");
    trace_call("moltis_auth_remove_passkey");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<AuthPasskeyIdRequest>(
            "moltis_auth_remove_passkey",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.remove_passkey(request.id))
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error("moltis_auth_remove_passkey", "AUTH_PASSKEY_REMOVE_FAILED");
                encode_error("AUTH_PASSKEY_REMOVE_FAILED", &error.to_string())
            },
        }
    })
}

/// Renames a passkey.
#[unsafe(no_mangle)]
pub extern "C" fn moltis_auth_rename_passkey(request_json: *const c_char) -> *mut c_char {
    record_call("moltis_auth_rename_passkey");
    trace_call("moltis_auth_rename_passkey");

    with_ffi_boundary(|| {
        let request = match parse_ffi_request::<AuthPasskeyRenameRequest>(
            "moltis_auth_rename_passkey",
            request_json,
        ) {
            Ok(r) => r,
            Err(e) => return e,
        };

        let name = request.name.trim();
        if name.is_empty() {
            record_error("moltis_auth_rename_passkey", "AUTH_PASSKEY_NAME_REQUIRED");
            return encode_error("AUTH_PASSKEY_NAME_REQUIRED", "name cannot be empty");
        }

        match BRIDGE
            .runtime
            .block_on(BRIDGE.credential_store.rename_passkey(request.id, name))
        {
            Ok(()) => encode_json(&OkResponse { ok: true }),
            Err(error) => {
                record_error("moltis_auth_rename_passkey", "AUTH_PASSKEY_RENAME_FAILED");
                encode_error("AUTH_PASSKEY_RENAME_FAILED", &error.to_string())
            },
        }
    })
}

// ── Tests ────────────────────────────────────────────────────────────────

#[allow(unsafe_code)]
#[cfg(test)]
mod tests {
    use std::ffi::{CString, c_char};

    use serde_json::Value;

    use super::*;

    fn text_from_ptr(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null(), "ffi returned null pointer");
        let owned = unsafe { CString::from_raw(ptr) };
        match owned.into_string() {
            Ok(text) => text,
            Err(error) => panic!("failed to decode UTF-8 from ffi pointer: {error}"),
        }
    }

    fn json_from_ptr(ptr: *mut c_char) -> Value {
        let text = text_from_ptr(ptr);
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(error) => panic!("failed to parse ffi json payload: {error}; payload={text}"),
        }
    }

    #[test]
    fn auth_status_returns_expected_fields() {
        let payload = json_from_ptr(moltis_auth_status());

        assert!(
            payload
                .get("auth_disabled")
                .and_then(Value::as_bool)
                .is_some(),
            "auth_status should return auth_disabled"
        );
        assert!(
            payload
                .get("has_password")
                .and_then(Value::as_bool)
                .is_some(),
            "auth_status should return has_password"
        );
        assert!(
            payload
                .get("has_passkeys")
                .and_then(Value::as_bool)
                .is_some(),
            "auth_status should return has_passkeys"
        );
        assert!(
            payload
                .get("setup_complete")
                .and_then(Value::as_bool)
                .is_some(),
            "auth_status should return setup_complete"
        );
    }

    #[test]
    fn auth_list_passkeys_returns_array() {
        let payload = json_from_ptr(moltis_auth_list_passkeys());
        assert!(
            payload.get("passkeys").and_then(Value::as_array).is_some(),
            "auth_list_passkeys should return passkeys"
        );
    }

    #[test]
    fn auth_password_change_rejects_short_password() {
        let request = r#"{"new_password":"short"}"#;
        let c_request = CString::new(request).unwrap_or_else(|e| panic!("{e}"));
        let payload = json_from_ptr(moltis_auth_password_change(c_request.as_ptr()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "AUTH_PASSWORD_TOO_SHORT");
    }

    #[test]
    fn auth_remove_passkey_returns_error_for_null() {
        let payload = json_from_ptr(moltis_auth_remove_passkey(std::ptr::null()));

        let code = payload
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(code, "null_pointer_or_invalid_utf8");
    }
}
