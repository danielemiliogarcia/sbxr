use crate::cli::SignSelection;
use crate::process;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const REBASE_SIGN_COMMAND: &str = "git -c core.hooksPath=/dev/null -c gpg.format=openpgp commit --amend --allow-empty --no-edit --no-verify -S";

struct Commit {
    id: String,
    parents: Vec<String>,
    author_email: String,
    summary: String,
    openpgp_signed: bool,
}

struct RewritePlan {
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

    let repository = repository_root(&requested)?;
    reject_in_progress_operation(&repository)?;
    match selection {
        SignSelection::Head => sign_head(&repository),
        SignSelection::AllMine | SignSelection::Range(_) => {
            sign_history(&repository, selection, approved)
        }
    }
}

fn sign_head(repository: &Path) -> Result<(), String> {
    let old_head = resolve_revision(repository, "HEAD")?;
    reject_staged_changes(repository)?;

    let summary = commit_summary(repository, &old_head)?;
    println!("==> OpenPGP-signing host commit {summary}");
    println!("    repository: {}", repository.display());
    println!("    Git hooks are disabled for this signing-only amend.");

    let status = git(repository)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "gpg.format=openpgp",
            "commit",
            "--amend",
            "--no-edit",
            "--no-verify",
            "-S",
        ])
        .status()
        .map_err(|error| format!("could not run host Git OpenPGP signing: {error}"))?;
    if !status.success() {
        return Err(signing_failure(status));
    }

    let new_head = resolve_revision(repository, "HEAD")?;
    verify_openpgp_commit(repository, &new_head)?;
    println!("signed: {old_head} -> {new_head}");
    println!(
        "verify: git -C {} verify-commit HEAD",
        shell_quote(repository)
    );
    Ok(())
}

fn sign_history(
    repository: &Path,
    selection: &SignSelection,
    approved: bool,
) -> Result<(), String> {
    reject_dirty_worktree(repository)?;
    let Some(plan) = build_rewrite_plan(repository, selection)? else {
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

    let backup = create_backup(repository, &plan.old_head)?;
    println!("==> recovery ref created: {backup}");
    println!(
        "==> rewriting and OpenPGP-signing {} commit(s)",
        plan.commits.len()
    );
    let status = git(repository)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "gpg.format=openpgp",
            "-c",
            "commit.gpgSign=false",
            "rebase",
            "--force-rebase",
            "--empty=keep",
            "--exec",
            REBASE_SIGN_COMMAND,
            &plan.base,
        ])
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", ":")
        .status()
        .map_err(|error| format!("could not start host Git history signing: {error}"))?;
    if !status.success() {
        let aborted = git(repository)
            .args(["-c", "core.hooksPath=/dev/null", "rebase", "--abort"])
            .status()
            .is_ok_and(|status| status.success());
        return Err(format!(
            "host OpenPGP history signing failed with {status}. {}\nRecovery ref: {backup}\nVerify the YubiKey, PIN/touch prompt, and Git signing configuration, then retry.",
            if aborted {
                "The rebase was aborted and the original branch was restored."
            } else {
                "Git could not abort the rebase automatically; inspect `git status` before continuing."
            }
        ));
    }

    let new_head = verify_rewrite(repository, &plan, &backup)?;
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
    repository: &Path,
    selection: &SignSelection,
) -> Result<Option<RewritePlan>, String> {
    let old_head = resolve_revision(repository, "HEAD")?;
    let branch = git_text(
        repository,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        "resolving the current branch",
    )
    .map_err(|_| {
        "history signing requires an attached branch; detached HEAD is not supported".to_owned()
    })?;
    let author_email = git_text(
        repository,
        &["config", "--get", "user.email"],
        "reading the effective Git user.email",
    )
    .map_err(|_| {
        "history signing requires an effective Git user.email to identify your commits".to_owned()
    })?;

    let (boundary, range_base) = match selection {
        SignSelection::AllMine => {
            let upstream = git_text(
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
            let base = resolve_revision(repository, &upstream)?;
            (upstream, base)
        }
        SignSelection::Range(range) => {
            let (left, right) = parse_explicit_range(range)?;
            let base = resolve_revision(repository, left)?;
            let selected_head = resolve_revision(repository, right)?;
            if selected_head != old_head {
                return Err(format!(
                    "--range must end at the current HEAD; {right} resolves to {selected_head}"
                ));
            }
            (range.clone(), base)
        }
        SignSelection::Head => unreachable!(),
    };
    require_ancestor(repository, &range_base, &old_head)?;

    let range = format!("{range_base}..{old_head}");
    let ids = git_lines(
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
        candidates.push(read_commit(repository, &id)?);
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
    let old_tree = resolve_object(repository, "HEAD^{tree}")?;
    Ok(Some(RewritePlan {
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

fn create_backup(repository: &Path, old_head: &str) -> Result<String, String> {
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

fn verify_rewrite(repository: &Path, plan: &RewritePlan, backup: &str) -> Result<String, String> {
    let new_head = resolve_revision(repository, "HEAD")?;
    let new_tree = resolve_object(repository, "HEAD^{tree}")?;
    if new_tree != plan.old_tree {
        return Err(format!(
            "history signing completed, but the HEAD tree changed unexpectedly; inspect the branch before pushing\nRecovery ref: {backup}"
        ));
    }
    let range = format!("{}..{new_head}", plan.base);
    let ids = git_lines(
        repository,
        &["rev-list", "--reverse", "--topo-order", &range],
        "verifying rewritten commits",
    )?;
    if ids.len() != plan.commits.len() {
        return Err(format!(
            "history signing rewrote an unexpected number of commits; expected {}, found {}\nRecovery ref: {backup}",
            plan.commits.len(),
            ids.len()
        ));
    }
    for id in ids {
        let commit = read_commit(repository, &id)?;
        if !author_matches(&commit.author_email, &plan.author_email) {
            return Err(format!(
                "rewritten commit {} has an unexpected author; inspect before pushing\nRecovery ref: {backup}",
                commit.summary
            ));
        }
        verify_openpgp_commit(repository, &id).map_err(|error| {
            format!("{error}\nThe rewritten branch remains available for inspection.\nRecovery ref: {backup}")
        })?;
    }
    Ok(new_head)
}

fn read_commit(repository: &Path, id: &str) -> Result<Commit, String> {
    let parents = git_text_allow_empty(
        repository,
        &["show", "-s", "--format=%P", id],
        "reading commit parents",
    )?
    .split_whitespace()
    .map(ToOwned::to_owned)
    .collect::<Vec<_>>();
    let author_email = git_text(
        repository,
        &["show", "-s", "--format=%ae", id],
        "reading commit author",
    )?;
    let summary = commit_summary(repository, id)?;
    let bytes = git_bytes(
        repository,
        &["cat-file", "commit", id],
        "reading the commit object",
    )?;
    Ok(Commit {
        id: id.to_owned(),
        parents,
        author_email,
        summary,
        openpgp_signed: has_openpgp_signature(&bytes),
    })
}

fn commit_summary(repository: &Path, id: &str) -> Result<String, String> {
    git_text(
        repository,
        &["show", "-s", "--format=%h %s", id],
        "reading the commit summary",
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

fn resolve_revision(repository: &Path, revision: &str) -> Result<String, String> {
    if revision.starts_with('-') || revision.chars().any(char::is_whitespace) {
        return Err(format!("unsafe Git revision: {revision}"));
    }
    let revision = format!("{revision}^{{commit}}");
    git_text(
        repository,
        &["rev-parse", "--verify", "--end-of-options", &revision],
        "resolving a Git revision",
    )
}

fn resolve_object(repository: &Path, revision: &str) -> Result<String, String> {
    if revision.starts_with('-') || revision.chars().any(char::is_whitespace) {
        return Err(format!("unsafe Git object revision: {revision}"));
    }
    git_text(
        repository,
        &["rev-parse", "--verify", "--end-of-options", revision],
        "resolving a Git object",
    )
}

fn require_ancestor(repository: &Path, base: &str, head: &str) -> Result<(), String> {
    let status = git(repository)
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

fn reject_dirty_worktree(repository: &Path) -> Result<(), String> {
    let output = git_bytes(
        repository,
        &["status", "--porcelain=v1", "--untracked-files=normal", "-z"],
        "checking the host worktree before history signing",
    )?;
    if output.is_empty() {
        Ok(())
    } else {
        Err("history signing requires a completely clean worktree and index, including no untracked files".to_owned())
    }
}

fn reject_staged_changes(repository: &Path) -> Result<(), String> {
    let status = git(repository)
        .args(["diff", "--cached", "--quiet", "HEAD", "--"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("could not inspect staged host changes: {error}"))?;
    match status.code() {
        Some(0) => Ok(()),
        Some(1) => Err(
            "refusing to sign while the index has staged changes; commit or unstage them first so signing cannot silently alter the commit tree"
                .to_owned(),
        ),
        _ => Err(format!(
            "could not compare the host index with HEAD: {status}"
        )),
    }
}

fn reject_in_progress_operation(repository: &Path) -> Result<(), String> {
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "REBASE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ] {
        let path = git_text(
            repository,
            &["rev-parse", "--git-path", marker],
            "inspecting the host Git operation state",
        )?;
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            repository.join(path)
        };
        if path.exists() {
            return Err(format!(
                "refusing to amend history while a Git operation is in progress ({marker})"
            ));
        }
    }
    Ok(())
}

fn repository_root(requested: &Path) -> Result<PathBuf, String> {
    let root = git_text(
        requested,
        &["rev-parse", "--show-toplevel"],
        "locating the host Git repository",
    )?;
    PathBuf::from(root)
        .canonicalize()
        .map_err(|error| format!("host Git reported an unavailable repository root: {error}"))
}

fn verify_openpgp_commit(repository: &Path, id: &str) -> Result<(), String> {
    let commit = git_bytes(
        repository,
        &["cat-file", "commit", id],
        "checking the signed commit",
    )?;
    if !has_openpgp_signature(&commit) {
        return Err(format!(
            "commit {id} has no OpenPGP signature after signing"
        ));
    }
    let status = git(repository)
        .args(["-c", "gpg.format=openpgp", "verify-commit", id])
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

fn git(repository: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(repository);
    command
}

fn git_run_checked(repository: &Path, args: &[&str], description: &str) -> Result<(), String> {
    let status = git(repository)
        .args(args)
        .status()
        .map_err(|error| format!("could not run {description}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{description} failed with {status}"))
    }
}

fn git_lines(repository: &Path, args: &[&str], description: &str) -> Result<Vec<String>, String> {
    let text = git_text_allow_empty(repository, args, description)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn git_text(repository: &Path, args: &[&str], description: &str) -> Result<String, String> {
    let text = git_text_allow_empty(repository, args, description)?;
    if text.is_empty() {
        Err(format!("{description} returned an empty value"))
    } else {
        Ok(text)
    }
}

fn git_text_allow_empty(
    repository: &Path,
    args: &[&str],
    description: &str,
) -> Result<String, String> {
    let output = git_bytes(repository, args, description)?;
    let text = String::from_utf8(output)
        .map_err(|error| format!("{description} returned non-UTF-8 output: {error}"))?;
    Ok(text.trim().to_owned())
}

fn git_bytes(repository: &Path, args: &[&str], description: &str) -> Result<Vec<u8>, String> {
    let output = git(repository)
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
