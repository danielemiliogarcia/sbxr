use crate::environment::Context;
use crate::{envfile, process, sbx};
use serde_json::Value;

const VERSION: &str = "1.4.247";
const X64_SHA512_BASE64: &str =
    "ivVcVrtn7nXvjBWlduQpvekKdY0i6SYfQaZRlmc3h3anN+BPv0tLC6sI6jRsv9ecOzCrMpqYp+pxNXgebsQLGA==";
const ARM64_SHA512_BASE64: &str =
    "bsI3xfKZzAeoj2JnQRGwcDLhF4u81uQpPf6BdBCXb4LC1n9ujVUXdtsqAbSSgA5y02oOkflpl8CZc+TzlmonBQ==";

pub(crate) fn ensure_installed(context: &Context, sandbox_name: &str) -> Result<(), String> {
    let script = installation_script();
    println!("==> installing checksum-pinned Orca {VERSION} (optional integration)");
    process::run_checked(
        sbx::exec_command(context.preserve_xdg_state).args([
            "-u",
            "root",
            sandbox_name,
            "bash",
            "-lc",
            &script,
        ]),
        "installing the Orca remote server",
    )
}

fn installation_script() -> String {
    format!(
        r#"set -euo pipefail
install -d -o agent -g agent -m 0700 \
  /home/agent/.config/orca \
  /home/agent/.local/share/orca \
  /home/agent/.local/state/sbxr
chown -R agent:agent \
  /home/agent/.config/orca \
  /home/agent/.local/share/orca \
  /home/agent/.local/state/sbxr
root=/opt/sbxr-orca/{VERSION}
if [ -x "$root/squashfs-root/AppRun" ] \
  && [ -x /usr/local/bin/sbxr-orca-server ] \
  && command -v xvfb-run >/dev/null 2>&1 \
  && command -v xauth >/dev/null 2>&1; then
  exit 0
fi
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  ca-certificates curl file jq xauth xvfb zlib1g-dev \
  libasound2t64 libgtk-3-0t64 libnotify4 libnss3 libsecret-1-0 \
  libxss1 libxtst6 xdg-utils
rm -rf /var/lib/apt/lists/*
if [ ! -x "$root/squashfs-root/AppRun" ]; then
case "$(uname -m)" in
  x86_64|amd64) asset=orca-linux.AppImage; digest='{X64_SHA512_BASE64}' ;;
  aarch64|arm64) asset=orca-linux-arm64.AppImage; digest='{ARM64_SHA512_BASE64}' ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
temporary=/tmp/orca.AppImage
curl --proto '=https' --tlsv1.2 -fsSL -o "$temporary" \
  "https://github.com/stablyai/orca/releases/download/v{VERSION}/$asset"
actual=$(sha512sum "$temporary" | cut -d' ' -f1)
expected=$(printf '%s' "$digest" | base64 -d | od -An -tx1 | tr -d ' \n')
test "$actual" = "$expected"
chmod 0755 "$temporary"
rm -rf "$root"
mkdir -p "$root"
cd "$root"
"$temporary" --appimage-extract >/dev/null
rm -f "$temporary"
test -x "$root/squashfs-root/AppRun"
fi
cat > /usr/local/bin/sbxr-orca-server <<'SH'
#!/bin/sh
set -eu
exec xvfb-run -a /opt/sbxr-orca/{VERSION}/squashfs-root/AppRun "$@"
SH
chmod 0755 /usr/local/bin/sbxr-orca-server
"#
    )
}

pub(crate) fn pairing_link(context: &Context, sandbox_name: &str) -> Result<String, String> {
    let script = r#"set -euo pipefail
state=/home/agent/.local/state/sbxr
log="$state/orca-server.log"
pidfile="$state/orca-server.pid"
mkdir -p "$state"
if [ -s "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
  :
else
  rm -f "$log" "$pidfile"
  nohup /usr/local/bin/sbxr-orca-server serve --port 6768 --pairing-address 127.0.0.1 >"$log" 2>&1 </dev/null &
  printf '%s\n' "$!" > "$pidfile"
fi
for attempt in $(seq 1 60); do
  link=$(grep -Eo '(orca|https?)://[^[:space:]]+' "$log" 2>/dev/null | tail -n1 || true)
  if [ -n "$link" ]; then printf '%s\n' "$link"; exit 0; fi
  sleep 1
done
echo 'Orca did not print a pairing link; recent log follows:' >&2
tail -n 20 "$log" >&2 || true
exit 1
"#;
    let output = process::output_checked(
        sbx::exec_command(context.preserve_xdg_state)
            .arg(sandbox_name)
            .args(["--", "bash", "-lc", script]),
        "starting the Orca remote server",
    )?;
    let link = String::from_utf8(output.stdout)
        .map_err(|error| format!("Orca pairing link is not UTF-8: {error}"))?
        .trim()
        .to_owned();
    if link.is_empty() {
        return Err("Orca returned an empty pairing link".to_owned());
    }
    let port = envfile::orca_host_port(sandbox_name);
    rewrite_pairing_endpoint(&link, port)
}

fn rewrite_pairing_endpoint(link: &str, port: u16) -> Result<String, String> {
    let Some((prefix, encoded_tail)) = link.split_once("code=") else {
        return Ok(link
            .replace("127.0.0.1:6768", &format!("127.0.0.1:{port}"))
            .replace("127.0.0.1%3A6768", &format!("127.0.0.1%3A{port}")));
    };
    let (encoded, suffix) = encoded_tail
        .split_once('&')
        .map_or((encoded_tail, ""), |(value, suffix)| (value, suffix));
    let decoded = base64_url_decode(encoded)?;
    let mut payload: Value = serde_json::from_slice(&decoded)
        .map_err(|error| format!("Orca pairing payload is not valid JSON: {error}"))?;
    let endpoint = payload
        .get("endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| "Orca pairing payload has no string endpoint".to_owned())?;
    let rewritten = endpoint.replace("127.0.0.1:6768", &format!("127.0.0.1:{port}"));
    if rewritten == endpoint {
        return Err(format!(
            "Orca pairing endpoint does not target the expected guest listener: {endpoint}"
        ));
    }
    payload["endpoint"] = Value::String(rewritten);
    let serialized = serde_json::to_vec(&payload)
        .map_err(|error| format!("could not serialize Orca pairing payload: {error}"))?;
    let separator = if suffix.is_empty() { "" } else { "&" };
    Ok(format!(
        "{prefix}code={}{}{}",
        base64_url_encode(&serialized),
        separator,
        suffix
    ))
}

fn base64_url_decode(encoded: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(encoded.len() * 3 / 4);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    let mut padding = false;
    for byte in encoded.bytes() {
        if byte == b'=' {
            padding = true;
            continue;
        }
        if padding {
            return Err("invalid base64url padding in Orca pairing payload".to_owned());
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err("invalid base64url character in Orca pairing payload".to_owned()),
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1_u32 << bits) - 1;
        }
    }
    if bits >= 6 || accumulator != 0 {
        return Err("invalid trailing base64url bits in Orca pairing payload".to_owned());
    }
    Ok(output)
}

fn base64_url_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    for byte in bytes {
        accumulator = (accumulator << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            output.push(ALPHABET[((accumulator >> bits) & 0x3f) as usize] as char);
            accumulator &= (1_u32 << bits) - 1;
        }
    }
    if bits != 0 {
        output.push(ALPHABET[((accumulator << (6 - bits)) & 0x3f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        base64_url_decode, base64_url_encode, installation_script, rewrite_pairing_endpoint,
    };
    use serde_json::{Value, json};

    #[test]
    fn rewrites_the_endpoint_inside_an_orca_pairing_payload() {
        let payload = json!({
            "v": 2,
            "endpoint": "ws://127.0.0.1:6768",
            "deviceToken": "not-a-real-token"
        });
        let encoded = base64_url_encode(&serde_json::to_vec(&payload).unwrap());
        let link = format!("orca://pair?code={encoded}");

        let rewritten = rewrite_pairing_endpoint(&link, 45678).unwrap();
        let encoded = rewritten.split_once("code=").unwrap().1;
        let decoded = base64_url_decode(encoded).unwrap();
        let decoded: Value = serde_json::from_slice(&decoded).unwrap();

        assert_eq!(decoded["endpoint"], "ws://127.0.0.1:45678");
        assert_eq!(decoded["deviceToken"], "not-a-real-token");
    }

    #[test]
    fn base64_url_codec_round_trips_unpadded_inputs() {
        for value in [b"".as_slice(), b"f", b"fo", b"foo", b"hello world"] {
            let encoded = base64_url_encode(value);
            assert!(!encoded.contains('='));
            assert_eq!(base64_url_decode(&encoded).unwrap(), value);
        }
    }

    #[test]
    fn installation_requires_and_repairs_xauth_without_redownloading_orca() {
        let script = installation_script();

        assert!(script.contains("command -v xauth"));
        assert!(script.contains("curl file jq xauth xvfb"));
        assert!(script.contains("if [ ! -x \"$root/squashfs-root/AppRun\" ]; then"));
    }
}
