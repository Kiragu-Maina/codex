use std::collections::BTreeMap;
use std::env;

use super::DoctorCheck;
use super::LOCALE_ENV_VARS;

const EDITOR_ENV_VARS: &[&str] = &["VISUAL", "EDITOR"];
const PAGER_ENV_VARS: &[&str] = &["PAGER", "GIT_PAGER", "GH_PAGER", "LESS"];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SystemCheckInputs {
    os: String,
    os_type: String,
    os_version: String,
    os_language: Option<String>,
    locale_env: BTreeMap<String, String>,
    editor_env: BTreeMap<String, String>,
    pager_env: BTreeMap<String, String>,
    user_resolution_detail: Option<String>,
    user_resolution_warning: Option<String>,
    fd_limit_detail: Option<String>,
    fd_limit_warning: Option<String>,
}

impl SystemCheckInputs {
    fn detect() -> Self {
        let info = os_info::get();
        let locale_env = LOCALE_ENV_VARS
            .iter()
            .filter_map(|name| {
                env::var(name)
                    .ok()
                    .map(|value| ((*name).to_string(), value))
            })
            .collect();
        let editor_env = EDITOR_ENV_VARS
            .iter()
            .map(|name| {
                let value = env::var_os(name)
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "not set".to_string());
                ((*name).to_string(), value)
            })
            .collect();
        let pager_env = PAGER_ENV_VARS
            .iter()
            .filter_map(|name| {
                env::var_os(name)
                    .map(|value| ((*name).to_string(), value.to_string_lossy().into_owned()))
            })
            .collect();
        #[cfg(unix)]
        let (user_resolution_detail, user_resolution_warning) = {
            let uid = unsafe { libc::getuid() };
            let pw = unsafe { libc::getpwuid(uid) };
            if pw.is_null() {
                (
                    Some(format!("user lookup: failed for uid {uid}")),
                    Some(format!(
                        "Local user lookup failed for UID {uid}. On macOS, this indicates degraded Directory Services / opendirectoryd IPC in the launch context."
                    )),
                )
            } else {
                let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }
                    .to_string_lossy()
                    .into_owned();
                (Some(format!("user: {name} (uid {uid})")), None)
            }
        };
        #[cfg(not(unix))]
        let (user_resolution_detail, user_resolution_warning) = (None, None);

        #[cfg(unix)]
        let (fd_limit_detail, fd_limit_warning) = {
            let mut rlim = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) } == 0 {
                let cur = rlim.rlim_cur;
                let max = rlim.rlim_max;
                if cur <= 256 {
                    (
                        Some(format!("open files limit: soft {cur}, hard {max} (low)")),
                        Some(format!(
                            "Open file limit is {cur} (default macOS launchd limit). Codex daemon and MCP servers typically require 300+ file descriptors."
                        )),
                    )
                } else {
                    (Some(format!("open files limit: soft {cur}, hard {max}")), None)
                }
            } else {
                (None, None)
            }
        };
        #[cfg(not(unix))]
        let (fd_limit_detail, fd_limit_warning) = (None, None);

        Self {
            os: info.to_string(),
            os_type: info.os_type().to_string(),
            os_version: info.version().to_string(),
            os_language: sys_locale::get_locale(),
            locale_env,
            editor_env,
            pager_env,
            user_resolution_detail,
            user_resolution_warning,
            fd_limit_detail,
            fd_limit_warning,
        }
    }
}

pub(super) fn system_check() -> DoctorCheck {
    system_check_from_inputs(SystemCheckInputs::detect())
}

fn system_check_from_inputs(inputs: SystemCheckInputs) -> DoctorCheck {
    let mut details = vec![
        format!("os: {}", inputs.os),
        format!("os type: {}", inputs.os_type),
        format!("os version: {}", inputs.os_version),
    ];
    if let Some(language) = inputs.os_language.as_deref() {
        details.push(format!("os language: {language}"));
    } else {
        details.push("os language: unavailable".to_string());
    }
    for name in LOCALE_ENV_VARS {
        if let Some(value) = inputs.locale_env.get(*name) {
            details.push(format!("{name}: {value}"));
        }
    }
    for name in EDITOR_ENV_VARS {
        if let Some(value) = inputs.editor_env.get(*name) {
            details.push(format!("{name}: {value}"));
        }
    }
    for name in PAGER_ENV_VARS {
        if let Some(value) = inputs.pager_env.get(*name) {
            details.push(format!("{name}: {value}"));
        }
    }
    if let Some(user_detail) = inputs.user_resolution_detail {
        details.push(user_detail);
    }
    if let Some(fd_detail) = inputs.fd_limit_detail {
        details.push(fd_detail);
    }

    let warning_message = inputs
        .user_resolution_warning
        .as_ref()
        .or(inputs.fd_limit_warning.as_ref());

    let summary = if let Some(warning) = warning_message {
        warning.clone()
    } else {
        inputs
            .os_language
            .as_deref()
            .map(|language| format!("OS language {language}"))
            .unwrap_or_else(|| "OS language unavailable".to_string())
    };

    let status = if warning_message.is_some() {
        super::CheckStatus::Warning
    } else {
        super::CheckStatus::Ok
    };

    let mut check = DoctorCheck::new("system.environment", "system", status, summary).details(details);
    if inputs.user_resolution_warning.is_some() {
        check = check.remediation("Try restarting the background daemon with: codex app-server daemon restart");
    } else if inputs.fd_limit_warning.is_some() {
        check = check.remediation("Raise open files limit with 'ulimit -n 10240' before launching codex");
    }
    check
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn system_check_reports_os_language_locale_editor_and_pager_env() {
        let mut locale_env = BTreeMap::new();
        locale_env.insert("LANG".to_string(), "en_US.UTF-8".to_string());
        let editor_env = BTreeMap::from([
            ("EDITOR".to_string(), "vim".to_string()),
            ("VISUAL".to_string(), "code --wait".to_string()),
        ]);
        let pager_env = BTreeMap::from([
            ("GH_PAGER".to_string(), "less".to_string()),
            ("GIT_PAGER".to_string(), "delta".to_string()),
            ("LESS".to_string(), "-FRX".to_string()),
            ("PAGER".to_string(), "less -R".to_string()),
        ]);
        let check = system_check_from_inputs(SystemCheckInputs {
            os: "macOS 15.0".to_string(),
            os_type: "macos".to_string(),
            os_version: "15.0".to_string(),
            os_language: Some("en-US".to_string()),
            locale_env,
            editor_env,
            pager_env,
            user_resolution_detail: None,
            user_resolution_warning: None,
            fd_limit_detail: None,
            fd_limit_warning: None,
        });

        assert_eq!(check.summary, "OS language en-US");
        assert_eq!(
            check.details,
            vec![
                "os: macOS 15.0",
                "os type: macos",
                "os version: 15.0",
                "os language: en-US",
                "LANG: en_US.UTF-8",
                "VISUAL: code --wait",
                "EDITOR: vim",
                "PAGER: less -R",
                "GIT_PAGER: delta",
                "GH_PAGER: less",
                "LESS: -FRX",
            ]
        );
    }

    #[test]
    fn system_check_handles_missing_os_language() {
        let check = system_check_from_inputs(SystemCheckInputs {
            os: "Linux".to_string(),
            os_type: "linux".to_string(),
            os_version: "unknown".to_string(),
            os_language: None,
            locale_env: BTreeMap::new(),
            editor_env: BTreeMap::from([
                ("EDITOR".to_string(), "not set".to_string()),
                ("VISUAL".to_string(), "not set".to_string()),
            ]),
            pager_env: BTreeMap::new(),
            user_resolution_detail: None,
            user_resolution_warning: None,
            fd_limit_detail: None,
            fd_limit_warning: None,
        });

        assert_eq!(check.summary, "OS language unavailable");
        assert_eq!(
            check.details,
            vec![
                "os: Linux",
                "os type: linux",
                "os version: unknown",
                "os language: unavailable",
                "VISUAL: not set",
                "EDITOR: not set",
            ]
        );
    }
}
