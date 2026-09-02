use std::process::Command;

pub const REQUIRED_API_VERSION: u32 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenCastInfo {
    pub version: u32,
    pub compatible: bool,
}

pub fn discover() -> Result<ScreenCastInfo, String> {
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--timeout=3",
            "--dest",
            "org.gnome.Mutter.ScreenCast",
            "--object-path",
            "/org/gnome/Mutter/ScreenCast",
            "--method",
            "org.freedesktop.DBus.Properties.Get",
            "org.gnome.Mutter.ScreenCast",
            "Version",
        ])
        .output()
        .map_err(|error| format!("cannot execute gdbus: {error}"))?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim().replace(['\r', '\n'], " ");
        return Err(if detail.is_empty() {
            format!("gdbus exited with {}", output.status)
        } else {
            format!("gdbus failed: {detail}")
        });
    }

    let stdout = std::str::from_utf8(&output.stdout)
        .map_err(|_| "gdbus returned non-UTF-8 output".to_owned())?;
    let version = parse_version_output(stdout)?;
    Ok(ScreenCastInfo {
        version,
        compatible: version >= REQUIRED_API_VERSION,
    })
}

fn parse_version_output(output: &str) -> Result<u32, String> {
    let value = output
        .trim()
        .strip_prefix("(<")
        .and_then(|value| value.strip_suffix(">,)"))
        .ok_or_else(|| format!("unexpected gdbus Version response: {output:?}"))?;
    let value = value.strip_prefix("uint32 ").unwrap_or(value);
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("invalid ScreenCast version value: {value:?}"));
    }
    value
        .parse::<u32>()
        .map_err(|_| format!("ScreenCast version is out of range: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gdbus_variant_formats() {
        assert_eq!(parse_version_output("(<4>,)\n").unwrap(), 4);
        assert_eq!(parse_version_output("(<uint32 12>,)").unwrap(), 12);
    }

    #[test]
    fn rejects_ambiguous_or_out_of_range_versions() {
        assert!(parse_version_output("(4,)").is_err());
        assert!(parse_version_output("(<4 extra>,)").is_err());
        assert!(parse_version_output("(<4294967296>,)").is_err());
    }
}
