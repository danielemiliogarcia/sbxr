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
