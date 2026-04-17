use crate::service::LaunchSpec;

pub(crate) fn render_launch_agent(spec: &LaunchSpec) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
            "<plist version=\"1.0\">\n",
            "<dict>\n",
            "  <key>Label</key>\n",
            "  <string>dev.execmanager.daemon</string>\n",
            "  <key>ProgramArguments</key>\n",
            "  <array>\n",
            "    <string>{}</string>\n",
            "    <string>daemon</string>\n",
            "    <string>run</string>\n",
            "  </array>\n",
            "  <key>WorkingDirectory</key>\n",
            "  <string>{}</string>\n",
            "  <key>EnvironmentVariables</key>\n",
            "  <dict>\n",
            "    <key>EXECMANAGER_CONFIG_DIR</key>\n",
            "    <string>{}</string>\n",
            "    <key>EXECMANAGER_RUNTIME_DIR</key>\n",
            "    <string>{}</string>\n",
            "    <key>EXECMANAGER_STATE_DIR</key>\n",
            "    <string>{}</string>\n",
            "  </dict>\n",
            "  <key>RunAtLoad</key>\n",
            "  <true/>\n",
            "  <key>KeepAlive</key>\n",
            "  <true/>\n",
            "</dict>\n",
            "</plist>\n"
        ),
        escape_xml(&spec.execmanager_path.display().to_string()),
        escape_xml(&spec.config_dir.display().to_string()),
        escape_xml(&spec.config_dir.display().to_string()),
        escape_xml(&spec.runtime_dir.display().to_string()),
        escape_xml(&spec.state_dir.display().to_string()),
    )
}

fn escape_xml(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(ch),
        }
    }

    escaped
}
