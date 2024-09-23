use std::collections::HashMap;
use std::env::{self, VarError};
use std::ffi::CString;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use super::utils::env::find_in_path;
use anyhow::{Context, Result};
use log::debug;

/// Automatically pass these environment variables to the microVM, if they are
/// set.
const WELL_KNOWN_ENV_VARS: [&str; 5] = [
    "LD_LIBRARY_PATH",
    "LIBGL_DRIVERS_PATH",
    "MESA_LOADER_DRIVER_OVERRIDE", // needed for asahi
    "PATH",                        // needed by `krun-guest` program
    "RUST_LOG",
];

/// See https://github.com/AsahiLinux/docs/wiki/Devices
const ASAHI_SOC_COMPAT_IDS: [&str; 1] = ["apple,arm-platform"];

pub fn prepare_vm_env_vars(env: Vec<(String, Option<String>)>) -> Result<HashMap<String, String>> {
    let mut env_map = HashMap::new();

    for key in WELL_KNOWN_ENV_VARS {
        let value = match env::var(key) {
            Ok(value) => value,
            Err(VarError::NotPresent) => {
                if key == "MESA_LOADER_DRIVER_OVERRIDE" {
                    match fs::read_to_string("/proc/device-tree/compatible") {
                        Ok(compatible) => {
                            for compat_id in compatible.split('\0') {
                                if ASAHI_SOC_COMPAT_IDS.iter().any(|&s| s == compat_id) {
                                    env_map.insert(
                                        "MESA_LOADER_DRIVER_OVERRIDE".to_owned(),
                                        "asahi".to_owned(),
                                    );
                                    break;
                                }
                            }
                        },
                        Err(err) if err.kind() == ErrorKind::NotFound => {
                            continue;
                        },
                        Err(err) => {
                            Err(err).context("Failed to read `/proc/device-tree/compatible`")?
                        },
                    }
                }
                continue;
            },
            Err(err) => Err(err).with_context(|| format!("Failed to get `{key}` env var"))?,
        };
        env_map.insert(key.to_owned(), value);
    }

    for (key, value) in env {
        let value = value.map_or_else(
            || env::var(&key).with_context(|| format!("Failed to get `{key}` env var")),
            Ok,
        )?;
        env_map.insert(key, value);
    }

    // If we have an X11 display in the host, set HOST_DISPLAY in the guest.
    // krun-guest will then use this to set up xauth and replace it with :1
    // (which is forwarded to the host display).
    if let Ok(display) = env::var("DISPLAY") {
        env_map.insert("HOST_DISPLAY".to_string(), display);

        // And forward XAUTHORITY. This will be modified to fix the
        // display name in krun-guest.
        if let Ok(xauthority) = env::var("XAUTHORITY") {
            env_map.insert("XAUTHORITY".to_string(), xauthority);
        }
    }

    debug!(env:? = env_map; "env vars");

    Ok(env_map)
}

const DROP_ENV_VARS: [&str; 17] = [
    "DBUS_SESSION_BUS_ADDRESS",
    "DISPLAY",
    "ICEAUTHORITY",
    "KONSOLE_DBUS_SERVICE",
    "KONSOLE_DBUS_SESSION",
    "KONSOLE_DBUS_WINDOW",
    "MANAGERPID",
    "PAM_KWALLET5_LOGIN",
    "SESSION_MANAGER",
    "SYSTEMD_EXEC_PID",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
    "XDG_SEAT",
    "XDG_SEAT_PATH",
    "XDG_SESSION_PATH",
    "XDG_VTNR",
];
pub fn prepare_proc_env_vars(env: Vec<(String, Option<String>)>) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for (k, v) in env::vars() {
        vars.insert(k, v);
    }
    for (k, v) in env {
        if let Some(v) = v {
            vars.insert(k, v);
        }
    }
    for k in DROP_ENV_VARS {
        vars.remove(k);
    }
    vars
}

pub fn find_krun_exec<P>(program: P) -> Result<CString>
where
    P: AsRef<Path>,
{
    let program = program.as_ref();
    let path = find_in_path(program)
        .with_context(|| format!("Failed to check existence of {program:?}"))?;
    let path = if let Some(path) = path {
        path
    } else {
        let path = env::current_exe().and_then(|p| p.canonicalize());
        let path = path.context("Failed to get path of current running executable")?;
        path.with_file_name(program)
    };
    let path = CString::new(path.to_str().with_context(|| {
        format!("Failed to process {program:?} path as it contains invalid UTF-8")
    })?)
    .with_context(|| format!("Failed to process {program:?} path as it contains NUL character"))?;

    Ok(path)
}
