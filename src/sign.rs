use crate::cli::SignSelection;
use crate::git_trust::{self, HostConfig, RepositoryLayout};
use crate::process;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct Commit {
    id: String,
    tree: String,
    parents: Vec<String>,
    author_name: String,
    author_email: String,
    author_date: String,
    encoding: Option<String>,
    message: Vec<u8>,
    summary: String,
    openpgp_signed: bool,
}

struct RewritePlan {
    branch_ref: String,
    branch: String,
    author_email: String,
    boundary: String,
    base: String,
    old_head: String,
    old_tree: String,
    commits: Vec<Commit>,
    originally_unsigned: usize,
}

pub(crate) fn commit_pgp_sign(
    requested: &Path,
    selection: &SignSelection,
    approved: bool,
) -> Result<(), String> {
    process::require("git")?;
    let requested = requested.canonicalize().map_err(|error| {
        format!(
            "project path {} is unavailable: {error}",
            requested.display()
        )
    })?;
    if !requested.is_dir() {
        return Err(format!(
            "project path is not a directory: {}",
            requested.display()
        ));
    }

    let trusted = HostConfig::load_for_signing(&requested)?;
    let audit = git_trust::audit(&requested, Some(&trusted.signing_key))?
        .ok_or_else(|| format!("no Git repository contains {}", requested.display()))?;
    trusted.ensure_programs_outside(&audit.layout.root)?;
    audit.print();
    if !audit.signing_allowed() {
        return Err(audit.rejection());
    }
    let repository = &audit.layout.root;
    reject_in_progress_operation(&audit.layout)?;
    match selection {
        SignSelection::Head => sign_head(&trusted, repository),
        SignSelection::AllMine | SignSelection::Range(_) => {
            sign_history(&trusted, repository, selection, approved)
        }
    }
}

fn sign_head(trusted: &HostConfig, repository: &Path) -> Result<(), String> {
    let old_head = resolve_revision(trusted, repository, "HEAD")?;
    let commit = read_commit(trusted, repository, &old_head)?;
    reject_staged_changes(trusted, repository, &commit.tree)?;

    println!("==> OpenPGP-signing host commit {}", commit.summary);
    println!("    repository: {}", repository.display());
    println!("    Git hooks and repository-controlled executable config are disabled.");

    let new_head = sign_commit(trusted, repository, &commit, &commit.parents)?;
    verify_openpgp_commit(trusted, repository, &new_head)?;
    let reference = current_reference(trusted, repository).unwrap_or_else(|_| "HEAD".to_owned());
    update_reference(trusted, repository, &reference, &new_head, &old_head)?;
    println!("signed: {old_head} -> {new_head}");
    println!(
        "verify: git -C {} verify-commit HEAD",
        shell_quote(repository)
    );
    Ok(())
}

fn sign_history(
    trusted: &HostConfig,
    repository: &Path,
    selection: &SignSelection,
    approved: bool,
) -> Result<(), String> {
    let old_head = resolve_revision(trusted, repository, "HEAD")?;
    let old_tree = resolve_object(trusted, repository, "HEAD^{tree}")?;
    reject_staged_changes(trusted, repository, &old_tree)?;
    let Some(plan) = build_rewrite_plan(trusted, repository, selection)? else {
        println!("all matching commits are already OpenPGP-signed; no history was changed");
        return Ok(());
    };
    print_rewrite_plan(repository, &plan);
    io::stdout()
        .flush()
        .map_err(|error| format!("could not display the history-signing plan: {error}"))?;
    if !approved && !confirm_rewrite()? {
        println!("cancelled; no commits were changed");
        return Ok(());
    }

    let backup = create_backup(trusted, repository, &plan.old_head)?;
    println!("==> recovery ref created: {backup}");
    println!(
        "==> rewriting and OpenPGP-signing {} commit(s)",
        plan.commits.len()
    );
    let mut parent = plan.base.clone();
    for commit in &plan.commits {
        let signed = sign_commit(trusted, repository, commit, &[parent])
            .map_err(|error| format!("{error}\nRecovery ref: {backup}"))?;
        verify_openpgp_commit(trusted, repository, &signed)
            .map_err(|error| format!("{error}\nRecovery ref: {backup}"))?;
        parent = signed;
    }
    let new_head = parent;
    let new_tree = resolve_object(trusted, repository, &format!("{new_head}^{{tree}}"))?;
    if new_tree != plan.old_tree {
        return Err(format!(
            "signed history has an unexpected final tree; the branch was not changed\nRecovery ref: {backup}"
        ));
    }
    update_reference(trusted, repository, &plan.branch_ref, &new_head, &old_head)
        .map_err(|error| format!("{error}\nRecovery ref: {backup}"))?;
    println!("signed {} commit(s)", plan.commits.len());
    println!("branch: {}", plan.branch);
    println!("old HEAD: {}", plan.old_head);
    println!("new HEAD: {new_head}");
    println!("recovery ref: {backup}");
    println!(
        "verify: git -C {} log --show-signature {}..HEAD",
        shell_quote(repository),
        plan.base
    );
    println!(
        "No push was performed. If the old history was already published, review it and use `git push --force-with-lease` explicitly."
    );
    Ok(())
}

fn build_rewrite_plan(
    trusted: &HostConfig,
    repository: &Path,
    selection: &SignSelection,
) -> Result<Option<RewritePlan>, String> {
    let old_head = resolve_revision(trusted, repository, "HEAD")?;
    let branch_ref = git_text(
        trusted,
        repository,
        &["symbolic-ref", "--quiet", "HEAD"],
        "resolving the current branch",
    )
    .map_err(|_| {
        "history signing requires an attached branch; detached HEAD is not supported".to_owned()
    })?;
    let branch = branch_ref
        .strip_prefix("refs/heads/")
        .unwrap_or(&branch_ref)
        .to_owned();
    let author_email = trusted.user_email.clone();

    let (boundary, range_base) = match selection {
        SignSelection::AllMine => {
            let upstream = git_text(
                trusted,
                repository,
                &[
                    "rev-parse",
                    "--abbrev-ref",
                    "--symbolic-full-name",
                    "@{upstream}",
                ],
                "resolving the current branch upstream",
            )
            .map_err(|_| {
                "--all-mine requires a configured upstream; use --range BASE..HEAD when the branch has no upstream"
                    .to_owned()
            })?;
            let base = resolve_revision(trusted, repository, &upstream)?;
            (upstream, base)
        }
        SignSelection::Range(range) => {
            let (left, right) = parse_explicit_range(range)?;
            let base = resolve_revision(trusted, repository, left)?;
            let selected_head = resolve_revision(trusted, repository, right)?;
            if selected_head != old_head {
                return Err(format!(
                    "--range must end at the current HEAD; {right} resolves to {selected_head}"
                ));
            }
            (range.clone(), base)
        }
        SignSelection::Head => unreachable!(),
    };
    require_ancestor(trusted, repository, &range_base, &old_head)?;

    let range = format!("{range_base}..{old_head}");
    let ids = git_lines(
        trusted,
        repository,
        &["rev-list", "--reverse", "--topo-order", &range],
        "listing commits selected for history signing",
    )?;
    if ids.is_empty() {
        return Err(format!(
            "no commits exist between {boundary} and the current HEAD"
        ));
    }
    let mut candidates = Vec::with_capacity(ids.len());
    for id in ids {
        candidates.push(read_commit(trusted, repository, &id)?);
    }
    let Some(first_unsigned) = candidates.iter().position(|commit| {
        author_matches(&commit.author_email, &author_email) && !commit.openpgp_signed
    }) else {
        return Ok(None);
    };
    let commits = candidates.split_off(first_unsigned);
    validate_linear_owned_suffix(&commits, &author_email)?;
    let base = commits[0].parents[0].clone();
    let originally_unsigned = commits
        .iter()
        .filter(|commit| !commit.openpgp_signed)
        .count();
    let old_tree = resolve_object(trusted, repository, "HEAD^{tree}")?;
    Ok(Some(RewritePlan {
        branch_ref,
        branch,
        author_email,
        boundary,
        base,
        old_head,
        old_tree,
        commits,
        originally_unsigned,
    }))
}

fn validate_linear_owned_suffix(commits: &[Commit], author_email: &str) -> Result<(), String> {
    for (index, commit) in commits.iter().enumerate() {
        if !author_matches(&commit.author_email, author_email) {
            return Err(format!(
                "refusing to rewrite commit {} because its author email is {}; every commit from the earliest unsigned commit through HEAD must belong to {}",
                commit.summary, commit.author_email, author_email
            ));
        }
        if commit.parents.len() != 1 {
            return Err(format!(
                "refusing to rewrite non-linear history at {}; merge and root commits are not supported",
                commit.summary
            ));
        }
        if index > 0 && commit.parents[0] != commits[index - 1].id {
            return Err(format!(
                "refusing to rewrite non-linear history at {}; use a linear branch suffix",
                commit.summary
            ));
        }
    }
    Ok(())
}

fn print_rewrite_plan(repository: &Path, plan: &RewritePlan) {
    println!("==> OpenPGP history-signing plan");
    println!("    repository: {}", repository.display());
    println!("    branch: {}", plan.branch);
    println!("    author: {}", plan.author_email);
    println!("    selection: {}", plan.boundary);
    println!(
        "    rewrite: {} commit(s), including {} originally unsigned",
        plan.commits.len(),
        plan.originally_unsigned
    );
    println!("    Git hooks are disabled; no push will be performed.");
    println!("    This is not a fast-forward: commit IDs will change.");
    for commit in &plan.commits {
        println!(
            "      {} {}",
            if commit.openpgp_signed {
                "re-sign"
            } else {
                "sign   "
            },
            commit.summary
        );
    }
}

fn confirm_rewrite() -> Result<bool, String> {
    eprint!("Rewrite this history and sign every listed commit? (y/N) ");
    io::stderr()
        .flush()
        .map_err(|error| format!("could not display history-signing confirmation: {error}"))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("could not read history-signing confirmation: {error}"))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn create_backup(
    trusted: &HostConfig,
    repository: &Path,
    old_head: &str,
) -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("host clock is before the Unix epoch: {error}"))?
        .as_secs();
    let short = old_head.get(..12).unwrap_or(old_head);
    let reference = format!(
        "refs/sbxr/commit-pgp-sign/{seconds}-{}-{short}",
        std::process::id()
    );
    git_run_checked(
        trusted,
        repository,
        &[
            "update-ref",
            "-m",
            "sbxr commit-pgp-sign recovery",
            &reference,
            old_head,
        ],
        "creating the host history-signing recovery ref",
    )?;
    Ok(reference)
}

fn read_commit(trusted: &HostConfig, repository: &Path, id: &str) -> Result<Commit, String> {
    let bytes = git_bytes(
        trusted,
        repository,
        &["cat-file", "commit", id],
        "reading the commit object",
    )?;
    parse_commit(id, &bytes)
}

fn parse_commit(id: &str, bytes: &[u8]) -> Result<Commit, String> {
    let separator = bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .ok_or_else(|| format!("commit {id} has no header separator"))?;
    let headers = &bytes[..separator];
    let message = bytes[separator + 2..].to_vec();
    let mut tree = None;
    let mut parents = Vec::new();
    let mut author = None;
    let mut encoding = None;
    let mut continuation = false;
    for line in headers.split(|byte| *byte == b'\n') {
        if line.first() == Some(&b' ') {
            if continuation {
                continue;
            }
            return Err(format!("commit {id} has an unexpected continued header"));
        }
        continuation = false;
        let separator = line
            .iter()
            .position(|byte| *byte == b' ')
            .ok_or_else(|| format!("commit {id} has a malformed header"))?;
        let (name, value_with_space) = line.split_at(separator);
        let value = &value_with_space[1..];
        match name {
            b"tree" => tree = Some(ascii_object_id(value, "tree", id)?),
            b"parent" => parents.push(ascii_object_id(value, "parent", id)?),
            b"author" => author = Some(parse_author(value, id)?),
            b"committer" => {}
            b"encoding" => {
                let value = std::str::from_utf8(value)
                    .map_err(|_| format!("commit {id} has a non-UTF-8 encoding header"))?;
                if value.is_empty()
                    || !value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
                {
                    return Err(format!("commit {id} has an unsafe encoding header"));
                }
                encoding = Some(value.to_owned());
            }
            b"gpgsig" | b"gpgsig-sha256" => continuation = true,
            _ => {
                return Err(format!(
                    "commit {id} contains unsupported header {}; refusing to reconstruct it",
                    String::from_utf8_lossy(name)
                ));
            }
        }
    }
    let tree = tree.ok_or_else(|| format!("commit {id} has no tree"))?;
    let (author_name, author_email, author_date) =
        author.ok_or_else(|| format!("commit {id} has no author"))?;
    let subject = String::from_utf8_lossy(&message)
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("<empty message>")
        .chars()
        .filter(|character| !character.is_control())
        .take(72)
        .collect::<String>();
    let short = id.get(..12).unwrap_or(id);
    Ok(Commit {
        id: id.to_owned(),
        tree,
        parents,
        author_name,
        author_email,
        author_date,
        encoding,
        message,
        summary: format!("{short} {subject}"),
        openpgp_signed: has_openpgp_signature(bytes),
    })
}

fn parse_author(value: &[u8], id: &str) -> Result<(String, String, String), String> {
    let value = std::str::from_utf8(value)
        .map_err(|_| format!("commit {id} has a non-UTF-8 author identity"))?;
    let email_end = value
        .rfind('>')
        .ok_or_else(|| format!("commit {id} has a malformed author identity"))?;
    let email_start = value[..email_end]
        .rfind('<')
        .ok_or_else(|| format!("commit {id} has a malformed author identity"))?;
    let name = value[..email_start].trim_end();
    let email = &value[email_start + 1..email_end];
    let mut date = value[email_end + 1..].split_whitespace();
    let timestamp = date
        .next()
        .ok_or_else(|| format!("commit {id} has no author timestamp"))?;
    let timezone = date
        .next()
        .ok_or_else(|| format!("commit {id} has no author timezone"))?;
    if date.next().is_some()
        || name.is_empty()
        || email.is_empty()
        || !timestamp.bytes().all(|byte| byte.is_ascii_digit())
        || timezone.len() != 5
        || !matches!(timezone.as_bytes()[0], b'+' | b'-')
        || !timezone.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(format!("commit {id} has an unsafe author identity or date"));
    }
    Ok((
        name.to_owned(),
        email.to_owned(),
        format!("@{timestamp} {timezone}"),
    ))
}

fn ascii_object_id(value: &[u8], header: &str, id: &str) -> Result<String, String> {
    let value =
        std::str::from_utf8(value).map_err(|_| format!("commit {id} has a non-ASCII {header}"))?;
    if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value.to_owned())
    } else {
        Err(format!("commit {id} has an invalid {header} object ID"))
    }
}

fn sign_commit(
    trusted: &HostConfig,
    repository: &Path,
    commit: &Commit,
    parents: &[String],
) -> Result<String, String> {
    let mut command = git(trusted, repository);
    command
        .arg("-c")
        .arg(format!(
            "i18n.commitEncoding={}",
            commit.encoding.as_deref().unwrap_or("UTF-8")
        ))
        .args(["commit-tree", &commit.tree]);
    for parent in parents {
        command.args(["-p", parent]);
    }
    command
        .arg("-S")
        .env("GIT_AUTHOR_NAME", &commit.author_name)
        .env("GIT_AUTHOR_EMAIL", &commit.author_email)
        .env("GIT_AUTHOR_DATE", &commit.author_date)
        .env("GIT_COMMITTER_NAME", &trusted.user_name)
        .env("GIT_COMMITTER_EMAIL", &trusted.user_email)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start host Git OpenPGP signing: {error}"))?;
    child
        .stdin
        .take()
        .ok_or("host Git signing has no message input")?
        .write_all(&commit.message)
        .map_err(|error| format!("could not supply the original commit message: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("could not finish host Git OpenPGP signing: {error}"))?;
    if !output.status.success() {
        return Err(signing_failure(output.status));
    }
    let id = String::from_utf8(output.stdout)
        .map_err(|_| "host Git signing returned a non-UTF-8 object ID".to_owned())?;
    let id = id.trim();
    if !matches!(id.len(), 40 | 64) || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("host Git signing returned an invalid object ID".to_owned());
    }
    Ok(id.to_owned())
}

fn current_reference(trusted: &HostConfig, repository: &Path) -> Result<String, String> {
    git_text(
        trusted,
        repository,
        &["symbolic-ref", "--quiet", "HEAD"],
        "resolving the current branch reference",
    )
}

fn update_reference(
    trusted: &HostConfig,
    repository: &Path,
    reference: &str,
    new: &str,
    old: &str,
) -> Result<(), String> {
    git_run_checked(
        trusted,
        repository,
        &[
            "update-ref",
            "-m",
            "sbxr commit-pgp-sign",
            reference,
            new,
            old,
        ],
        "atomically updating the signed host branch",
    )
}

fn parse_explicit_range(range: &str) -> Result<(&str, &str), String> {
    if range.starts_with('-') || range.chars().any(char::is_whitespace) || range.contains("...") {
        return Err("--range must use the form BASE..HEAD".to_owned());
    }
    let Some((left, right)) = range.split_once("..") else {
        return Err("--range must use the form BASE..HEAD".to_owned());
    };
    if left.is_empty() || right.is_empty() || right.contains("..") {
        return Err("--range must use the form BASE..HEAD".to_owned());
    }
    Ok((left, right))
}

fn resolve_revision(
    trusted: &HostConfig,
    repository: &Path,
    revision: &str,
) -> Result<String, String> {
    if revision.starts_with('-') || revision.chars().any(char::is_whitespace) {
        return Err(format!("unsafe Git revision: {revision}"));
    }
    let revision = format!("{revision}^{{commit}}");
    git_text(
        trusted,
        repository,
        &["rev-parse", "--verify", "--end-of-options", &revision],
        "resolving a Git revision",
    )
}

fn resolve_object(
    trusted: &HostConfig,
    repository: &Path,
    revision: &str,
) -> Result<String, String> {
    if revision.starts_with('-') || revision.chars().any(char::is_whitespace) {
        return Err(format!("unsafe Git object revision: {revision}"));
    }
    git_text(
        trusted,
        repository,
        &["rev-parse", "--verify", "--end-of-options", revision],
        "resolving a Git object",
    )
}

fn require_ancestor(
    trusted: &HostConfig,
    repository: &Path,
    base: &str,
    head: &str,
) -> Result<(), String> {
    let status = git(trusted, repository)
        .args(["merge-base", "--is-ancestor", base, head])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not compare the signing boundary with HEAD: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("the selected signing boundary is not an ancestor of HEAD; refusing a divergent history rewrite".to_owned())
    }
}

fn reject_staged_changes(
    trusted: &HostConfig,
    repository: &Path,
    expected_tree: &str,
) -> Result<(), String> {
    let index_tree = git_text(
        trusted,
        repository,
        &["write-tree"],
        "reading the host index tree before signing",
    )?;
    if index_tree == expected_tree {
        Ok(())
    } else {
        Err(
            "refusing to sign while the index has staged changes; commit or unstage them first so signing cannot silently alter the commit tree"
                .to_owned(),
        )
    }
}

fn reject_in_progress_operation(layout: &RepositoryLayout) -> Result<(), String> {
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "REBASE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ] {
        let in_git_dir = layout.git_dir.join(marker);
        let in_common_dir = layout.common_dir.join(marker);
        if in_git_dir.exists() || in_common_dir.exists() {
            return Err(format!(
                "refusing to amend history while a Git operation is in progress ({marker})"
            ));
        }
    }
    Ok(())
}

fn verify_openpgp_commit(trusted: &HostConfig, repository: &Path, id: &str) -> Result<(), String> {
    let commit = git_bytes(
        trusted,
        repository,
        &["cat-file", "commit", id],
        "checking the signed commit",
    )?;
    if !has_openpgp_signature(&commit) {
        return Err(format!(
            "commit {id} has no OpenPGP signature after signing"
        ));
    }
    let status = git(trusted, repository)
        .args(["verify-commit", id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not verify OpenPGP commit {id}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "commit {id} contains an OpenPGP signature, but cryptographic verification failed"
        ))
    }
}

fn author_matches(commit_email: &str, configured_email: &str) -> bool {
    commit_email.eq_ignore_ascii_case(configured_email)
}

fn signing_failure(status: ExitStatus) -> String {
    format!(
        "host OpenPGP signing failed with {status}; verify `gpg --card-status`, Git user.signingKey/gpg.program, the YubiKey PIN prompt, and touch the key when requested"
    )
}

fn git(trusted: &HostConfig, repository: &Path) -> Command {
    trusted.command(repository)
}

fn git_run_checked(
    trusted: &HostConfig,
    repository: &Path,
    args: &[&str],
    description: &str,
) -> Result<(), String> {
    let status = git(trusted, repository)
        .args(args)
        .status()
        .map_err(|error| format!("could not run {description}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{description} failed with {status}"))
    }
}

fn git_lines(
    trusted: &HostConfig,
    repository: &Path,
    args: &[&str],
    description: &str,
) -> Result<Vec<String>, String> {
    let text = git_text_allow_empty(trusted, repository, args, description)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn git_text(
    trusted: &HostConfig,
    repository: &Path,
    args: &[&str],
    description: &str,
) -> Result<String, String> {
    let text = git_text_allow_empty(trusted, repository, args, description)?;
    if text.is_empty() {
        Err(format!("{description} returned an empty value"))
    } else {
        Ok(text)
    }
}

fn git_text_allow_empty(
    trusted: &HostConfig,
    repository: &Path,
    args: &[&str],
    description: &str,
) -> Result<String, String> {
    let output = git_bytes(trusted, repository, args, description)?;
    let text = String::from_utf8(output)
        .map_err(|error| format!("{description} returned non-UTF-8 output: {error}"))?;
    Ok(text.trim().to_owned())
}

fn git_bytes(
    trusted: &HostConfig,
    repository: &Path,
    args: &[&str],
    description: &str,
) -> Result<Vec<u8>, String> {
    let output = git(trusted, repository)
        .args(args)
        .output()
        .map_err(|error| format!("could not run {description}: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(command_error(description, output.status, &stderr))
    }
}

fn command_error(description: &str, status: ExitStatus, stderr: &str) -> String {
    if stderr.is_empty() {
        format!("{description} failed with {status}")
    } else {
        format!("{description} failed: {stderr}")
    }
}

fn has_openpgp_signature(commit: &[u8]) -> bool {
    commit
        .windows(b"gpgsig -----BEGIN PGP SIGNATURE-----".len())
        .any(|window| window == b"gpgsig -----BEGIN PGP SIGNATURE-----")
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_openpgp_commit_headers() {
        assert!(has_openpgp_signature(
            b"tree abc\ngpgsig -----BEGIN PGP SIGNATURE-----\n value\n\nmessage\n"
        ));
        assert!(!has_openpgp_signature(
            b"tree abc\ngpgsig -----BEGIN SSH SIGNATURE-----\n value\n\nmessage\n"
        ));
        assert!(!has_openpgp_signature(b"tree abc\n\nmessage\n"));
    }

    #[test]
    fn parses_only_two_dot_explicit_ranges() {
        assert_eq!(
            parse_explicit_range("origin/main..HEAD").unwrap(),
            ("origin/main", "HEAD")
        );
        assert!(parse_explicit_range("origin/main...HEAD").is_err());
        assert!(parse_explicit_range("HEAD").is_err());
        assert!(parse_explicit_range("..HEAD").is_err());
        assert!(parse_explicit_range("--all..HEAD").is_err());
    }

    #[test]
    fn author_email_matching_is_ascii_case_insensitive() {
        assert!(author_matches("Emi@Example.com", "emi@example.com"));
        assert!(!author_matches("other@example.com", "emi@example.com"));
    }

    #[test]
    fn quotes_host_paths_for_display() {
        assert_eq!(shell_quote(Path::new("/tmp/a b's")), "'/tmp/a b'\\''s'");
    }
}

/// End-to-end signing through real `git commit-tree -S` with a throwaway
/// OpenPGP key, including a hostile repository whose executable settings and
/// hooks must stay inert even when the audit is bypassed.
#[cfg(all(test, unix))]
mod signing_integration {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    const SIGNER_NAME: &str = "sbxr test signer";
    const SIGNER_EMAIL: &str = "signer@example.invalid";
    const AUTHOR_DATE: &str = "1700000000 +0100";
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn short_temp(label: &str) -> PathBuf {
        // Kept under /tmp and short: gpg-agent socket paths have a length limit.
        PathBuf::from(format!(
            "/tmp/sbxr-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn write_executable(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn fixture_git(directory: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args([
                "-c",
                "commit.gpgSign=false",
                "-c",
                "init.defaultBranch=master",
            ])
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
            ])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn commit_as(repository: &Path, name: &str, email: &str, file: &str, message: &str) {
        fs::write(repository.join(file), format!("{message}\n")).unwrap();
        fixture_git(repository, &["add", file]);
        let status = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args([
                "-c",
                "commit.gpgSign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(["commit", "-q", "-m", message])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_AUTHOR_DATE", AUTHOR_DATE)
            .env("GIT_COMMITTER_NAME", name)
            .env("GIT_COMMITTER_EMAIL", email)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn header(repository: &Path, revision: &str, name: &str) -> String {
        fixture_git(repository, &["cat-file", "commit", revision])
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name} ")).map(str::to_owned))
            .unwrap_or_default()
    }

    /// Throwaway OpenPGP signer in an isolated GnuPG home.
    struct Signer {
        home: PathBuf,
        wrapper: PathBuf,
        fingerprint: String,
    }

    impl Signer {
        fn create() -> Option<Self> {
            let Some(gpg) = process::command_path("gpg") else {
                assert!(
                    std::env::var_os("SBXR_REQUIRE_GPG_TESTS").is_none(),
                    "gpg is required when SBXR_REQUIRE_GPG_TESTS is set"
                );
                eprintln!("skipping OpenPGP signing integration test: gpg is unavailable");
                return None;
            };
            let home = short_temp("gpg");
            fs::create_dir(&home).unwrap();
            fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
            // Constructed first so Drop cleans up the home on any failure below.
            let mut signer = Self {
                wrapper: home.join("gpg-wrapper"),
                home,
                fingerprint: String::new(),
            };
            let status = Command::new(&gpg)
                .arg("--homedir")
                .arg(&signer.home)
                .args(["--batch", "--pinentry-mode", "loopback", "--passphrase", ""])
                .args(["--quick-gen-key"])
                .arg(format!("{SIGNER_NAME} <{SIGNER_EMAIL}>"))
                .args(["ed25519", "sign", "never"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "throwaway OpenPGP key generation failed");
            let listing = Command::new(&gpg)
                .arg("--homedir")
                .arg(&signer.home)
                .args(["--batch", "--with-colons", "--list-secret-keys"])
                .output()
                .unwrap();
            signer.fingerprint = String::from_utf8(listing.stdout)
                .unwrap()
                .lines()
                .find_map(|line| {
                    line.strip_prefix("fpr:")
                        .and_then(|rest| rest.split(':').nth(8))
                        .map(str::to_owned)
                })
                .expect("generated key has a fingerprint");
            write_executable(
                &signer.wrapper,
                &format!(
                    "#!/bin/sh\nexec '{}' --homedir '{}' --batch \"$@\"\n",
                    gpg.display(),
                    signer.home.display()
                ),
            );
            Some(signer)
        }

        fn trusted(&self) -> HostConfig {
            HostConfig::for_tests(
                self.wrapper.clone(),
                SIGNER_NAME,
                SIGNER_EMAIL,
                &self.fingerprint,
            )
        }

        fn verifies(&self, repository: &Path, revision: &str) -> bool {
            Command::new("git")
                .arg("-C")
                .arg(repository)
                .arg("-c")
                .arg(format!("gpg.program={}", self.wrapper.display()))
                .args(["verify-commit", revision])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        }
    }

    impl Drop for Signer {
        fn drop(&mut self) {
            let _ = Command::new("gpgconf")
                .arg("--homedir")
                .arg(&self.home)
                .args(["--kill", "all"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    /// A branch with an upstream commit by someone else followed by two
    /// unsigned commits by the signer.
    struct Fixture {
        root: PathBuf,
        repository: PathBuf,
        base: String,
    }

    impl Fixture {
        fn create() -> Self {
            let root = short_temp("sign");
            let repository = root.join("repository");
            fs::create_dir_all(&repository).unwrap();
            fixture_git(&repository, &["init", "-q"]);
            commit_as(
                &repository,
                "Upstream",
                "upstream@example.invalid",
                "base.txt",
                "upstream: base",
            );
            fixture_git(&repository, &["branch", "upstream"]);
            fixture_git(&repository, &["branch", "--set-upstream-to=upstream"]);
            let base = fixture_git(&repository, &["rev-parse", "HEAD"]);
            commit_as(
                &repository,
                SIGNER_NAME,
                SIGNER_EMAIL,
                "one.txt",
                "feat: one",
            );
            commit_as(
                &repository,
                SIGNER_NAME,
                SIGNER_EMAIL,
                "two.txt",
                "feat: two",
            );
            Self {
                root,
                repository,
                base,
            }
        }

        fn head(&self) -> String {
            fixture_git(&self.repository, &["rev-parse", "HEAD"])
        }

        fn tree(&self, revision: &str) -> String {
            fixture_git(
                &self.repository,
                &["rev-parse", &format!("{revision}^{{tree}}")],
            )
        }

        /// Plants every repository-controlled execution vector the signer
        /// must neutralize. Each one appends to `marker` if it ever runs.
        fn make_hostile(&self, marker: &Path) -> PathBuf {
            let payload = self.root.join("payload.sh");
            write_executable(
                &payload,
                &format!(
                    "#!/bin/sh\necho \"$0 $*\" >> '{}'\nexit 1\n",
                    marker.display()
                ),
            );
            let payload_text = payload.to_str().unwrap();
            let alternate_hooks = self.root.join("alternate-hooks");
            fs::create_dir_all(&alternate_hooks).unwrap();
            for hook in [
                "pre-commit",
                "commit-msg",
                "post-commit",
                "post-rewrite",
                "post-checkout",
                "reference-transaction",
                "pre-auto-gc",
            ] {
                for directory in [self.repository.join(".git/hooks"), alternate_hooks.clone()] {
                    write_executable(
                        &directory.join(hook),
                        &format!("#!/bin/sh\n'{payload_text}' {hook}\n"),
                    );
                }
            }
            for (key, value) in [
                ("core.hooksPath", alternate_hooks.to_str().unwrap()),
                ("core.fsmonitor", payload_text),
                ("core.pager", payload_text),
                ("core.editor", payload_text),
                ("gpg.program", payload_text),
                ("gpg.openpgp.program", payload_text),
                ("gpg.format", "ssh"),
                ("gpg.ssh.program", payload_text),
                (
                    "user.signingKey",
                    "0000000000000000000000000000000000000000",
                ),
                ("user.name", "Attacker"),
                ("user.email", "attacker@example.invalid"),
                ("filter.evil.clean", payload_text),
                ("filter.evil.smudge", payload_text),
                ("filter.evil.required", "true"),
                ("gc.auto", "1"),
            ] {
                fixture_git(&self.repository, &["config", "--local", key, value]);
            }
            fs::write(self.repository.join(".gitattributes"), "* filter=evil\n").unwrap();
            payload
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn all_mine_reconstructs_signed_history_without_running_repository_code() {
        let Some(signer) = Signer::create() else {
            return;
        };
        let fixture = Fixture::create();
        let repository = &fixture.repository;
        let marker = fixture.root.join("HOST_COMMAND_EXECUTED");
        fixture.make_hostile(&marker);
        let old_head = fixture.head();
        let old_tree = fixture.tree("HEAD");
        let old_authors = [
            header(repository, "HEAD~1", "author"),
            header(repository, "HEAD", "author"),
        ];
        let old_messages = [
            fixture_git(repository, &["log", "-1", "--format=%B", "HEAD~1"]),
            fixture_git(repository, &["log", "-1", "--format=%B", "HEAD"]),
        ];

        // Bypass the audit on purpose: the trusted command alone must keep
        // hooks, fsmonitor, pagers, filters, and repository signers inert.
        sign_history(&signer.trusted(), repository, &SignSelection::AllMine, true).unwrap();

        assert!(
            !marker.exists(),
            "repository-controlled code ran: {}",
            fs::read_to_string(&marker).unwrap_or_default()
        );
        let new_head = fixture.head();
        assert_ne!(new_head, old_head);
        assert_eq!(fixture.tree("HEAD"), old_tree);
        assert_eq!(
            fixture_git(repository, &["rev-parse", "HEAD~2"]),
            fixture.base
        );
        for (index, revision) in ["HEAD~1", "HEAD"].into_iter().enumerate() {
            assert!(
                signer.verifies(repository, revision),
                "{revision} unverified"
            );
            assert_eq!(header(repository, revision, "author"), old_authors[index]);
            assert!(header(repository, revision, "author").contains(AUTHOR_DATE));
            let committer = header(repository, revision, "committer");
            assert!(committer.starts_with(&format!("{SIGNER_NAME} <{SIGNER_EMAIL}>")));
            assert_eq!(
                fixture_git(repository, &["log", "-1", "--format=%B", revision]),
                old_messages[index]
            );
            assert_eq!(
                fixture.tree(revision),
                fixture.tree(&format!("{old_head}{}", if index == 0 { "~1" } else { "" }))
            );
        }
        assert!(!signer.verifies(repository, "HEAD~2"));
        let recovery = fixture_git(
            repository,
            &[
                "for-each-ref",
                "--format=%(objectname)",
                "refs/sbxr/commit-pgp-sign",
            ],
        );
        assert_eq!(recovery, old_head);
    }

    #[test]
    fn head_signing_keeps_parents_tree_and_author() {
        let Some(signer) = Signer::create() else {
            return;
        };
        let fixture = Fixture::create();
        let repository = &fixture.repository;
        let old_head = fixture.head();
        let old_parent = fixture_git(repository, &["rev-parse", "HEAD~1"]);
        let old_author = header(repository, "HEAD", "author");

        sign_head(&signer.trusted(), repository).unwrap();

        assert_ne!(fixture.head(), old_head);
        assert!(signer.verifies(repository, "HEAD"));
        assert!(!signer.verifies(repository, "HEAD~1"));
        assert_eq!(
            fixture_git(repository, &["rev-parse", "HEAD~1"]),
            old_parent
        );
        assert_eq!(fixture.tree("HEAD"), fixture.tree(&old_head));
        assert_eq!(header(repository, "HEAD", "author"), old_author);
    }

    #[test]
    fn signing_failure_leaves_the_branch_unchanged() {
        let fixture = Fixture::create();
        let repository = &fixture.repository;
        let failing = fixture.root.join("failing-gpg");
        write_executable(&failing, "#!/bin/sh\nexit 1\n");
        let trusted = HostConfig::for_tests(failing, SIGNER_NAME, SIGNER_EMAIL, "ABCDEF");
        let old_head = fixture.head();

        assert!(sign_head(&trusted, repository).is_err());
        assert_eq!(fixture.head(), old_head);

        let error = sign_history(&trusted, repository, &SignSelection::AllMine, true).unwrap_err();
        assert!(error.contains("Recovery ref"), "{error}");
        assert_eq!(fixture.head(), old_head);
        assert_eq!(
            fixture_git(repository, &["symbolic-ref", "HEAD"]),
            "refs/heads/master"
        );
    }
}
