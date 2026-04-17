use crate::service::LaunchSpec;

pub(crate) fn render_unit(spec: &LaunchSpec) -> String {
    format!(
        concat!(
            "[Unit]\n",
            "Description=ExecManager per-user daemon\n\n",
            "[Service]\n",
            "Type=simple\n",
            "ExecStart={} daemon run\n",
            "WorkingDirectory={}\n",
            "Environment=EXECMANAGER_CONFIG_DIR={}\n",
            "Environment=EXECMANAGER_RUNTIME_DIR={}\n",
            "Environment=EXECMANAGER_STATE_DIR={}\n",
            "Restart=on-failure\n\n",
            "[Install]\n",
            "WantedBy=default.target\n"
        ),
        escape_systemd_value(&spec.execmanager_path.display().to_string()),
        escape_systemd_value(&spec.config_dir.display().to_string()),
        escape_systemd_value(&spec.config_dir.display().to_string()),
        escape_systemd_value(&spec.runtime_dir.display().to_string()),
        escape_systemd_value(&spec.state_dir.display().to_string()),
    )
}

fn escape_systemd_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            ' ' | '\t' | '\n' | '"' | '\'' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }

    escaped
}
