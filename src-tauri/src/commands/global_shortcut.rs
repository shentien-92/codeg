use sea_orm::DatabaseConnection;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::app_error::AppCommandError;
use crate::db::service::app_metadata_service;
use crate::db::AppDatabase;

const GLOBAL_SHORTCUT_DB_KEY: &str = "global_shortcut";

/// Default global shortcut in frontend format.
const DEFAULT_SHORTCUT_FRONTEND: &str = "alt+shift+g";

/// Convert from frontend shortcut format (`mod+shift+g`) to the
/// `global_hotkey` crate string format (`CmdOrCtrl+Shift+KeyG`).
///
/// Frontend tokens → global_hotkey tokens:
///   mod        → CmdOrCtrl
///   alt        → Alt
///   shift      → Shift
///   <letter>   → Key<UPPER>   (a → KeyA)
///   f1..f12    → F1..F12
///   space      → Space
///   escape     → Escape
///   enter      → Enter
///   tab        → Tab
///   backspace  → Backspace
///   delete     → Delete
///   arrowup    → ArrowUp
///   arrowdown  → ArrowDown
///   arrowleft  → ArrowLeft
///   arrowright → ArrowRight
fn frontend_to_tauri(frontend: &str) -> Result<String, AppCommandError> {
    let parts: Vec<&str> = frontend.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return Err(AppCommandError::invalid_input("Empty shortcut string"));
    }

    let mut result_parts: Vec<String> = Vec::new();

    for part in &parts {
        let lower = part.to_lowercase();
        match lower.as_str() {
            "mod" | "cmd" | "command" | "meta" | "ctrl" | "control" => {
                result_parts.push("CmdOrCtrl".to_string());
            }
            "alt" | "option" => {
                result_parts.push("Alt".to_string());
            }
            "shift" => {
                result_parts.push("Shift".to_string());
            }
            _ => {
                let key = map_key_to_code(&lower)?;
                result_parts.push(key);
            }
        }
    }

    Ok(result_parts.join("+"))
}

/// Convert from `global_hotkey` format (`CmdOrCtrl+Shift+KeyG`) back to
/// frontend format (`mod+shift+g`).
#[allow(dead_code)]
fn tauri_to_frontend(tauri_str: &str) -> String {
    let parts: Vec<&str> = tauri_str.split('+').map(str::trim).collect();
    let mut result_parts: Vec<String> = Vec::new();

    for part in &parts {
        let mapped = match *part {
            "CmdOrCtrl" | "CommandOrControl" | "Super" | "Control" | "Command" => {
                "mod".to_string()
            }
            "Alt" => "alt".to_string(),
            "Shift" => "shift".to_string(),
            other => map_code_to_key(other),
        };
        result_parts.push(mapped);
    }

    result_parts.join("+")
}

/// Map a frontend key token to a `global_hotkey` key code string.
fn map_key_to_code(key: &str) -> Result<String, AppCommandError> {
    // Single letter a-z
    if key.len() == 1 {
        let ch = key.chars().next().unwrap();
        if ch.is_ascii_alphabetic() {
            return Ok(format!("Key{}", ch.to_ascii_uppercase()));
        }
        if ch.is_ascii_digit() {
            return Ok(format!("Digit{ch}"));
        }
        // Special single-char keys
        return match ch {
            '[' => Ok("BracketLeft".to_string()),
            ']' => Ok("BracketRight".to_string()),
            '\\' => Ok("Backslash".to_string()),
            '/' => Ok("Slash".to_string()),
            ',' => Ok("Comma".to_string()),
            '.' => Ok("Period".to_string()),
            ';' => Ok("Semicolon".to_string()),
            '\'' => Ok("Quote".to_string()),
            '`' => Ok("Backquote".to_string()),
            '-' => Ok("Minus".to_string()),
            '=' => Ok("Equal".to_string()),
            _ => Err(AppCommandError::invalid_input(format!(
                "Unsupported key: {ch}"
            ))),
        };
    }

    // Function keys
    if key.starts_with('f') && key.len() <= 3 {
        if let Ok(n) = key[1..].parse::<u8>() {
            if (1..=24).contains(&n) {
                return Ok(format!("F{n}"));
            }
        }
    }

    match key {
        "space" => Ok("Space".to_string()),
        "escape" => Ok("Escape".to_string()),
        "enter" => Ok("Enter".to_string()),
        "tab" => Ok("Tab".to_string()),
        "backspace" => Ok("Backspace".to_string()),
        "delete" => Ok("Delete".to_string()),
        "arrowup" => Ok("ArrowUp".to_string()),
        "arrowdown" => Ok("ArrowDown".to_string()),
        "arrowleft" => Ok("ArrowLeft".to_string()),
        "arrowright" => Ok("ArrowRight".to_string()),
        _ => Err(AppCommandError::invalid_input(format!(
            "Unsupported key: {key}"
        ))),
    }
}

/// Map a `global_hotkey` code string back to a frontend key token.
#[allow(dead_code)]
fn map_code_to_key(code: &str) -> String {
    // KeyA..KeyZ
    if let Some(letter) = code.strip_prefix("Key") {
        return letter.to_lowercase();
    }
    // Digit0..Digit9
    if let Some(digit) = code.strip_prefix("Digit") {
        return digit.to_string();
    }
    // Function keys F1..F24
    if code.starts_with('F') && code.len() <= 3 && code[1..].parse::<u8>().is_ok() {
        return code.to_lowercase();
    }
    match code {
        "Space" => "space".to_string(),
        "Escape" => "escape".to_string(),
        "Enter" => "enter".to_string(),
        "Tab" => "tab".to_string(),
        "Backspace" => "backspace".to_string(),
        "Delete" => "delete".to_string(),
        "ArrowUp" => "arrowup".to_string(),
        "ArrowDown" => "arrowdown".to_string(),
        "ArrowLeft" => "arrowleft".to_string(),
        "ArrowRight" => "arrowright".to_string(),
        "BracketLeft" => "[".to_string(),
        "BracketRight" => "]".to_string(),
        "Backslash" => "\\".to_string(),
        "Slash" => "/".to_string(),
        "Comma" => ",".to_string(),
        "Period" => ".".to_string(),
        "Semicolon" => ";".to_string(),
        "Quote" => "'".to_string(),
        "Backquote" => "`".to_string(),
        "Minus" => "-".to_string(),
        "Equal" => "=".to_string(),
        _ => code.to_lowercase(),
    }
}

/// Toggle visibility of all application windows.
///
/// If any visible window exists, hide all windows. Otherwise, show and focus
/// all windows.
fn toggle_app_visibility(app: &AppHandle) {
    let windows = app.webview_windows();
    if windows.is_empty() {
        return;
    }

    // Check if at least one window is currently visible.
    let any_visible = windows.values().any(|w| w.is_visible().unwrap_or(false));

    if any_visible {
        for w in windows.values() {
            let _ = w.hide();
        }
    } else {
        for w in windows.values() {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

/// Load the saved global shortcut from the DB and register it.
/// Called once during application startup.
pub async fn load_and_register_global_shortcut(
    app: &AppHandle,
    conn: &DatabaseConnection,
) {
    let frontend_shortcut = match app_metadata_service::get_value(conn, GLOBAL_SHORTCUT_DB_KEY).await
    {
        Ok(Some(val)) if !val.is_empty() => val,
        _ => DEFAULT_SHORTCUT_FRONTEND.to_string(),
    };

    if let Err(e) = register_shortcut(app, &frontend_shortcut) {
        eprintln!(
            "[GlobalShortcut] failed to register shortcut '{}': {}",
            frontend_shortcut, e
        );
    } else {
        eprintln!(
            "[GlobalShortcut] registered global shortcut: {}",
            frontend_shortcut
        );
    }
}

/// Register a global shortcut that toggles app visibility.
fn register_shortcut(app: &AppHandle, frontend_shortcut: &str) -> Result<(), AppCommandError> {
    let tauri_str = frontend_to_tauri(frontend_shortcut)?;

    app.global_shortcut()
        .on_shortcut(tauri_str.as_str(), move |app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            toggle_app_visibility(app);
        })
        .map_err(|e| {
            AppCommandError::configuration_invalid("Failed to register global shortcut")
                .with_detail(e.to_string())
        })?;

    Ok(())
}

/// Unregister the currently active global shortcut.
fn unregister_shortcut(app: &AppHandle, frontend_shortcut: &str) -> Result<(), AppCommandError> {
    let tauri_str = frontend_to_tauri(frontend_shortcut)?;

    app.global_shortcut()
        .unregister(tauri_str.as_str())
        .map_err(|e| {
            AppCommandError::configuration_invalid("Failed to unregister global shortcut")
                .with_detail(e.to_string())
        })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Get the current global shortcut in frontend format.
#[tauri::command]
pub async fn get_global_shortcut(
    db: State<'_, AppDatabase>,
) -> Result<String, AppCommandError> {
    let val = app_metadata_service::get_value(&db.conn, GLOBAL_SHORTCUT_DB_KEY)
        .await
        .map_err(AppCommandError::from)?;

    Ok(val.unwrap_or_else(|| DEFAULT_SHORTCUT_FRONTEND.to_string()))
}

/// Update the global shortcut. Unregisters the old one and registers the new.
/// Returns the newly saved shortcut in frontend format.
#[tauri::command]
pub async fn update_global_shortcut(
    shortcut: String,
    db: State<'_, AppDatabase>,
    app: AppHandle,
) -> Result<String, AppCommandError> {
    let shortcut = shortcut.trim().to_lowercase();
    if shortcut.is_empty() {
        return Err(AppCommandError::invalid_input(
            "Shortcut string must not be empty",
        ));
    }

    // Validate the new shortcut by attempting to convert it.
    let _tauri_str = frontend_to_tauri(&shortcut)?;

    // Unregister the old shortcut (ignore errors if it wasn't registered).
    let old = app_metadata_service::get_value(&db.conn, GLOBAL_SHORTCUT_DB_KEY)
        .await
        .map_err(AppCommandError::from)?
        .unwrap_or_else(|| DEFAULT_SHORTCUT_FRONTEND.to_string());

    let _ = unregister_shortcut(&app, &old);

    // Register the new shortcut.
    register_shortcut(&app, &shortcut)?;

    // Persist.
    app_metadata_service::upsert_value(&db.conn, GLOBAL_SHORTCUT_DB_KEY, &shortcut)
        .await
        .map_err(AppCommandError::from)?;

    Ok(shortcut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frontend_to_tauri_basic() {
        assert_eq!(
            frontend_to_tauri("mod+shift+g").unwrap(),
            "CmdOrCtrl+Shift+KeyG"
        );
        assert_eq!(frontend_to_tauri("alt+shift+g").unwrap(), "Alt+Shift+KeyG");
        assert_eq!(frontend_to_tauri("mod+k").unwrap(), "CmdOrCtrl+KeyK");
        assert_eq!(frontend_to_tauri("mod+1").unwrap(), "CmdOrCtrl+Digit1");
        assert_eq!(frontend_to_tauri("alt+space").unwrap(), "Alt+Space");
        assert_eq!(frontend_to_tauri("mod+f1").unwrap(), "CmdOrCtrl+F1");
    }

    #[test]
    fn test_tauri_to_frontend_basic() {
        assert_eq!(tauri_to_frontend("CmdOrCtrl+Shift+KeyG"), "mod+shift+g");
        assert_eq!(tauri_to_frontend("Alt+Shift+KeyG"), "alt+shift+g");
        assert_eq!(tauri_to_frontend("CmdOrCtrl+KeyK"), "mod+k");
        assert_eq!(tauri_to_frontend("CmdOrCtrl+Digit1"), "mod+1");
        assert_eq!(tauri_to_frontend("Alt+Space"), "alt+space");
    }

    #[test]
    fn test_roundtrip() {
        let cases = vec![
            "mod+shift+g",
            "alt+shift+g",
            "mod+k",
            "alt+space",
            "mod+f12",
            "mod+shift+escape",
        ];
        for frontend in cases {
            let tauri_str = frontend_to_tauri(frontend).unwrap();
            let back = tauri_to_frontend(&tauri_str);
            assert_eq!(back, frontend, "roundtrip failed for: {}", frontend);
        }
    }
}
