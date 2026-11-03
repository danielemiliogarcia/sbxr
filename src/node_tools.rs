use crate::cli::Preset;
use crate::environment::Context;
use crate::profile::Profile;
use crate::{process, sbx, sha256};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

#[cfg(test)]
use std::path::PathBuf;

const MARKER: &str = "/home/agent/.local/state/sbxr/node-tools-v1.sha256";
const INSTALL_FORMAT: &[u8] = b"sbxr-locked-node-tools-v1\0";

#[derive(Clone, Copy)]
struct Tool {
    name: &'static str,
    kit: &'static str,
    install: &'static str,
    executable: &'static str,
}

type ToolAssets<'a> = (&'a Tool, Vec<u8>, Vec<u8>);

const CLAUDE_INSTALL: &str = r#"
set -euo pipefail
TOOL_ROOT=/opt/sbxr-tools/claude
npm ci --prefix "$TOOL_ROOT" --ignore-scripts --no-audit --no-fund
node - "$TOOL_ROOT" <<'NODE'
const fs = require('node:fs');
const path = require('node:path');
const toolRoot = process.argv[2];
const claudeRoot = path.join(toolRoot, 'node_modules/@anthropic-ai/claude-code');
const nativePackage = '@anthropic-ai/claude-code-linux-' + process.arch;
const source = path.join(path.dirname(require.resolve(nativePackage + '/package.json', {paths: [claudeRoot]})), 'claude');
fs.accessSync(source, fs.constants.X_OK);
const destination = path.join(claudeRoot, 'bin/claude.exe');
fs.rmSync(destination, {force: true});
fs.symlinkSync(source, destination);
NODE
REAL="$TOOL_ROOT/node_modules/@anthropic-ai/claude-code/bin/claude.exe"
SHIM=/usr/local/bin/claude
test -x "$REAL"
rm -f "$SHIM"
printf '%s\n' \
  '#!/bin/sh' \
  '# Prefer the Docker-managed OAuth credential file over API-key variables.' \
  'unset ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN ANTHROPIC_BASE_URL' \
  "exec \"$REAL\" \"\$@\"" > "$SHIM"
chmod 0755 "$SHIM"
claude --version
"#;

const PI_INSTALL: &str = r#"
set -euo pipefail
TOOL_ROOT=/opt/sbxr-tools/pi
node -e '
  const [major, minor] = process.versions.node.split(".").map(Number);
  if (major < 22 || (major === 22 && minor < 19)) {
    throw new Error(`Pi 1.4.0 requires Node >=22.19.0; found ${process.version}`);
  }
'
npm ci --prefix "$TOOL_ROOT" --ignore-scripts --no-audit --no-fund
ln -sfn "$TOOL_ROOT/node_modules/.bin/pi" /usr/local/bin/pi
PACKAGE="$TOOL_ROOT/node_modules/@earendil-works/pi-coding-agent"
node - "$PACKAGE" <<'NODE'
const fs = require("fs");
const path = require("path");
const packageDir = process.argv[2];
const targets = [
  path.join(packageDir, "node_modules/@earendil-works/pi-ai/dist/api/openai-codex-responses.js"),
  path.join(path.dirname(packageDir), "pi-ai/dist/api/openai-codex-responses.js")
].filter((file) => fs.existsSync(file));
const chunks = path.join(packageDir, "dist/bundle/chunks");
if (fs.existsSync(chunks)) {
  for (const file of fs.readdirSync(chunks)) {
    if (file.startsWith("openai-codex-responses-") && file.endsWith(".js")) {
      targets.push(path.join(chunks, file));
    }
  }
}
if (targets.length < 2) throw new Error("Pi proxy patch targets were not found");
for (const target of targets) {
  let source = fs.readFileSync(target, "utf8");
  if (source.includes('token === "oai-oat01-proxy-managed"') ||
      source.includes('token==="oai-oat01-proxy-managed"')) continue;
  const replacements = [
    [
      "function extractAccountId(token) {\n    try {",
      'function extractAccountId(token) {\n    if (token === "oai-oat01-proxy-managed") return "proxy-managed";\n    try {'
    ],
    [
      "function extractAccountId(token){try{",
      'function extractAccountId(token){if(token==="oai-oat01-proxy-managed")return"proxy-managed";try{'
    ]
  ];
  const replacement = replacements.find(([original]) => source.includes(original));
  if (!replacement) throw new Error(`Pi proxy patch target was not found in ${target}`);
  source = source.replace(replacement[0], replacement[1]);
  fs.writeFileSync(target, source);
}
NODE
mkdir -p /home/agent/.pi/agent
node <<'NODE'
const fs = require("fs");
const path = "/home/agent/.pi/agent/auth.json";
let auth = {};
try { auth = JSON.parse(fs.readFileSync(path, "utf8")); } catch {}
auth["openai-codex"] = {
  type: "oauth",
  access: "oai-oat01-proxy-managed",
  refresh: "proxy-managed",
  expires: 4102444800000,
  accountId: "proxy-managed",
  source: "docker-sbx-proxy"
};
auth.anthropic = {
  type: "oauth",
  access: "sk-ant-oat01-proxy-managed",
  refresh: "sk-ant-ort01-proxy-managed",
  expires: 4102444800000,
  source: "docker-sbx-proxy"
};
fs.writeFileSync(path, JSON.stringify(auth, null, 2) + "\n", { mode: 0o600 });
fs.chmodSync(path, 0o600);
NODE
chown -R agent:agent /home/agent/.pi
pi --version
"#;

const PASEO_INSTALL: &str = r#"
set -euo pipefail
TOOL_ROOT=/opt/sbxr-tools/paseo
npm ci --prefix "$TOOL_ROOT" --ignore-scripts --no-audit --no-fund
ln -sfn "$TOOL_ROOT/node_modules/.bin/paseo" /usr/local/bin/paseo
paseo --version
"#;

const BROWSER_INSTALL: &str = r#"
set -euo pipefail
TOOL_ROOT=/opt/sbxr-tools/browser
export PLAYWRIGHT_BROWSERS_PATH=/home/agent/.cache/ms-playwright
npm ci --prefix "$TOOL_ROOT" --ignore-scripts --no-audit --no-fund
node "$TOOL_ROOT/node_modules/playwright/cli.js" install-deps chromium
apt-get update
apt-get install -y --no-install-recommends libnss3-tools
rm -rf /var/lib/apt/lists/*
install -d -m 0700 -o agent -g agent "$PLAYWRIGHT_BROWSERS_PATH" \
  /home/agent/.local/share/pki/nssdb /home/agent/.cache/sbxr/browser-artifacts
if [ -r /usr/local/share/ca-certificates/proxy-ca.crt ]; then
  db=sql:/home/agent/.local/share/pki/nssdb
  sudo -u agent certutil -N -d "$db" --empty-password >/dev/null 2>&1 || true
  sudo -u agent certutil -D -d "$db" -n sbxr-proxy >/dev/null 2>&1 || true
  sudo -u agent certutil -A -d "$db" -n sbxr-proxy -t 'C,,' \
    -i /usr/local/share/ca-certificates/proxy-ca.crt
fi
sudo -u agent env HOME=/home/agent PLAYWRIGHT_BROWSERS_PATH="$PLAYWRIGHT_BROWSERS_PATH" \
  node "$TOOL_ROOT/node_modules/playwright/cli.js" install chromium
cat > /usr/local/bin/sbxr-browser-mcp <<'SH'
#!/bin/sh
set -eu
export PLAYWRIGHT_BROWSERS_PATH=/home/agent/.cache/ms-playwright
proxy=${HTTPS_PROXY:-${https_proxy:-${HTTP_PROXY:-${http_proxy:-}}}}
test -n "$proxy" || { echo 'sandbox HTTP proxy is required' >&2; exit 1; }
exec node /opt/sbxr-tools/browser/node_modules/@playwright/mcp/cli.js \
  --headless --isolated --browser chromium \
  --proxy-server "$proxy" --proxy-bypass 'localhost,127.0.0.1,[::1]' \
  --output-dir /home/agent/.cache/sbxr/browser-artifacts "$@"
SH
chmod 0755 /usr/local/bin/sbxr-browser-mcp
test -x /usr/local/bin/sbxr-browser-mcp
"#;

const CLAUDE: Tool = Tool {
    name: "Claude Code",
    kit: "claude-cli",
    install: CLAUDE_INSTALL,
    executable: "/usr/local/bin/claude",
};
const PI: Tool = Tool {
    name: "Pi",
    kit: "pi-cli",
    install: PI_INSTALL,
    executable: "/usr/local/bin/pi",
};
const PASEO: Tool = Tool {
    name: "Paseo",
    kit: "paseo-cli",
    install: PASEO_INSTALL,
    executable: "/usr/local/bin/paseo",
};
const BROWSER: Tool = Tool {
    name: "Playwright browser MCP",
    kit: "browser",
    install: BROWSER_INSTALL,
    executable: "/usr/local/bin/sbxr-browser-mcp",
};

pub(crate) fn ensure(
    context: &Context,
    sandbox_name: &str,
    preset: Preset,
    profile: &Profile,
) -> Result<(), String> {
    let mut tools = if preset == Preset::MultiAgent {
        vec![CLAUDE, PI, PASEO]
    } else {
        vec![PASEO]
    };
    if profile.browser {
        tools.push(BROWSER);
    }
    let assets = load_assets(context, &tools)?;
    let wanted = toolchain_hash(&tools, &assets);
    if installed(context, sandbox_name, &tools, &wanted)? {
        return Ok(());
    }

    println!("==> installing lockfile-pinned Node tools");
    for (tool, package, lock) in assets {
        let root = format!("/opt/sbxr-tools/{}", tool.kit.trim_end_matches("-cli"));
        upload_root_file(
            context,
            sandbox_name,
            &format!("{root}/package.json"),
            &package,
        )?;
        upload_root_file(
            context,
            sandbox_name,
            &format!("{root}/package-lock.json"),
            &lock,
        )?;
        println!("  - {}", tool.name);
        process::run_checked(
            sbx::exec_command(context.preserve_xdg_state).args([
                "-u",
                "root",
                sandbox_name,
                "bash",
                "-lc",
                tool.install,
            ]),
            &format!("installing lockfile-pinned {}", tool.name),
        )?;
    }
    sbx::write_file(
        context.preserve_xdg_state,
        sandbox_name,
        MARKER,
        OsStr::new(&wanted),
    )
}

fn load_assets<'a>(context: &Context, tools: &'a [Tool]) -> Result<Vec<ToolAssets<'a>>, String> {
    tools
        .iter()
        .map(|tool| {
            let root = context
                .kit_root
                .join(tool.kit)
                .join("files/opt/sbxr-tools")
                .join(tool.kit.trim_end_matches("-cli"));
            let package = read_asset(&root.join("package.json"))?;
            let lock = read_asset(&root.join("package-lock.json"))?;
            Ok((tool, package, lock))
        })
        .collect()
}

fn read_asset(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))
}

fn toolchain_hash(tools: &[Tool], assets: &[ToolAssets<'_>]) -> String {
    let mut input = INSTALL_FORMAT.to_vec();
    for (tool, package, lock) in assets {
        input.extend_from_slice(tool.kit.as_bytes());
        input.push(0);
        input.extend_from_slice(tool.install.as_bytes());
        input.push(0);
        input.extend_from_slice(package);
        input.push(0);
        input.extend_from_slice(lock);
        input.push(0);
    }
    debug_assert_eq!(tools.len(), assets.len());
    sha256::hex(&input)
}

fn installed(
    context: &Context,
    sandbox_name: &str,
    tools: &[Tool],
    wanted: &str,
) -> Result<bool, String> {
    let current = sbx::read_file(context.preserve_xdg_state, sandbox_name, MARKER)?;
    if current.as_deref().map(|value| value.trim_ascii()) != Some(wanted.as_bytes()) {
        return Ok(false);
    }
    let mut command = sbx::exec_command(context.preserve_xdg_state);
    command.args([
        sandbox_name,
        "sh",
        "-c",
        "for path do test -x \"$path\" || exit 1; done",
        "sh",
    ]);
    for tool in tools {
        command.arg(tool.executable);
    }
    Ok(command.status().is_ok_and(|status| status.success()))
}

fn upload_root_file(
    context: &Context,
    sandbox_name: &str,
    path: &str,
    contents: &[u8],
) -> Result<(), String> {
    let script = r#"set -eu
path=$1
directory=${path%/*}
mkdir -p "$directory"
temporary="$path.tmp.$$"
trap 'rm -f "$temporary"' EXIT HUP INT TERM
umask 022
cat > "$temporary"
chmod 0644 "$temporary"
mv -f "$temporary" "$path"
trap - EXIT HUP INT TERM
"#;
    let mut child = sbx::exec_command(context.preserve_xdg_state)
        .args([
            "-u",
            "root",
            "-i",
            sandbox_name,
            "sh",
            "-c",
            script,
            "sh",
            path,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not upload {path}: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| format!("sandbox upload stdin was unavailable for {path}"))?
        .write_all(contents)
        .map_err(|error| format!("could not stream {path}: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("could not finish uploading {path}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("uploading {path} failed with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolchain_hash_changes_with_assets_and_install_logic() {
        let tools = [PASEO];
        let first = vec![(&tools[0], b"package".to_vec(), b"lock".to_vec())];
        let second = vec![(&tools[0], b"package".to_vec(), b"lock2".to_vec())];
        assert_ne!(
            toolchain_hash(&tools, &first),
            toolchain_hash(&tools, &second)
        );
    }

    #[test]
    fn pi_install_always_reasserts_proxy_managed_authentication() {
        assert!(PI_INSTALL.contains("auth[\"openai-codex\"] ="));
        assert!(PI_INSTALL.contains("auth.anthropic ="));
        assert!(!PI_INSTALL.contains("if (!auth"));
    }

    #[test]
    fn vendored_tool_roots_match_guest_roots() {
        for tool in [CLAUDE, PI, PASEO] {
            let root =
                PathBuf::from("files/opt/sbxr-tools").join(tool.kit.trim_end_matches("-cli"));
            assert_eq!(
                root.file_name().and_then(|name| name.to_str()),
                Path::new(tool.executable)
                    .file_name()
                    .and_then(|name| name.to_str())
            );
        }
    }
}
