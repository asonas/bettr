pub fn current() -> Result<Option<crate::domain::WorktreeContext>, crate::error::AppError> {
    let path = std::env::current_dir().map_err(|error| {
        crate::error::AppError::Internal(format!("could not resolve current directory: {error}"))
    })?;
    resolve(&path, false)
}

pub fn from_path(
    path: &std::path::Path,
) -> Result<crate::domain::WorktreeContext, crate::error::AppError> {
    resolve(path, true)?.ok_or_else(|| {
        crate::error::AppError::InvalidInput(format!(
            "path is not inside a Git worktree: {}",
            path.display()
        ))
    })
}

fn resolve(
    path: &std::path::Path,
    strict: bool,
) -> Result<Option<crate::domain::WorktreeContext>, crate::error::AppError> {
    let path = match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(_error) if !strict => return Ok(None),
        Err(error) => {
            return Err(crate::error::AppError::InvalidInput(format!(
                "could not resolve Git worktree path {}: {error}",
                path.display()
            )));
        }
    };
    let Some(inside_worktree) = run_git(&path, &["rev-parse", "--is-inside-work-tree"])? else {
        return Ok(None);
    };
    if inside_worktree != "true" {
        return Ok(None);
    }
    let Some(root) = run_git(&path, &["rev-parse", "--show-toplevel"])? else {
        return Ok(None);
    };
    let root = std::fs::canonicalize(&root).map_err(|error| {
        crate::error::AppError::InvalidInput(format!(
            "could not resolve Git worktree root {root}: {error}"
        ))
    })?;
    let branch = run_git(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    Ok(Some(crate::domain::WorktreeContext {
        path: root.to_string_lossy().into_owned(),
        branch,
    }))
}

fn run_git(
    path: &std::path::Path,
    arguments: &[&str],
) -> Result<Option<String>, crate::error::AppError> {
    let output = match std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(crate::error::AppError::Internal(format!(
                "could not execute git: {error}"
            )));
        }
    };
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8(output.stdout).map_err(|_| {
        crate::error::AppError::InvalidInput("Git output must be valid Unicode".to_owned())
    })?;
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.to_owned()))
    }
}
