use keyring::{Entry, Error as KeyringError};

use crate::error::AppError;

const KEYCHAIN_SERVICE: &str = "inkwash-desktop";
const ADMIN_TOKEN_ACCOUNT: &str = "server-admin-token";

fn admin_token_entry() -> Result<Entry, AppError> {
    Entry::new(KEYCHAIN_SERVICE, ADMIN_TOKEN_ACCOUNT)
        .map_err(|err| AppError::internal(format!("open admin token keychain entry: {err}")))
}

#[tauri::command]
pub fn load_admin_token() -> Result<String, AppError> {
    match admin_token_entry()?.get_password() {
        Ok(token) => Ok(token),
        Err(KeyringError::NoEntry) => Ok(String::new()),
        Err(err) => Err(AppError::internal(format!(
            "read admin token from keychain: {err}"
        ))),
    }
}

#[tauri::command]
pub fn save_admin_token(token: String) -> Result<(), AppError> {
    let entry = admin_token_entry()?;
    if token.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(err) => Err(AppError::internal(format!(
                "delete admin token from keychain: {err}"
            ))),
        }
    } else {
        entry
            .set_password(&token)
            .map_err(|err| AppError::internal(format!("write admin token to keychain: {err}")))
    }
}
