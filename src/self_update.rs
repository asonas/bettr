const REPOSITORY: &str = "asonas/bettr";
const RELEASE_API_URL: &str = "https://api.github.com/repos/asonas/bettr/releases/latest";
const MAIN_API_URL: &str = "https://api.github.com/repos/asonas/bettr/commits/main";

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateResult {
    Updated,
    Installed,
    Failed,
}

impl UpdateResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Updated => "updated",
            Self::Installed => "installed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UninstallResult {
    Removed,
    NotInstalled,
    Failed,
}

impl UninstallResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::NotInstalled => "not_installed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ComponentUpdate {
    pub source: crate::app::UpdateSource,
    pub version: Option<String>,
    pub revision: String,
    pub result: UpdateResult,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct SelfUpdateReport {
    pub source: crate::app::UpdateSource,
    pub revision: String,
    pub cli: ComponentUpdate,
    pub codex: ComponentUpdate,
    pub claude: ComponentUpdate,
}

#[derive(Debug, serde::Serialize)]
pub struct ComponentUninstall {
    pub result: UninstallResult,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct SelfUninstallReport {
    pub codex: ComponentUninstall,
    pub claude: ComponentUninstall,
}

impl SelfUninstallReport {
    pub fn succeeded(&self) -> bool {
        [&self.codex.result, &self.claude.result]
            .into_iter()
            .all(|result| *result != UninstallResult::Failed)
    }
}

impl SelfUpdateReport {
    pub fn succeeded(&self) -> bool {
        [&self.cli.result, &self.codex.result, &self.claude.result]
            .into_iter()
            .all(|result| *result != UpdateResult::Failed)
    }
}

impl crate::app::UpdateSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Main => "main",
        }
    }
}

struct Bundle {
    _temporary_directory: TemporaryDirectory,
    root: std::path::PathBuf,
    binary: std::path::PathBuf,
    source: crate::app::UpdateSource,
    version: String,
    revision: String,
}

struct TemporaryDirectory {
    path: std::path::PathBuf,
}

impl TemporaryDirectory {
    fn new(prefix: &str) -> Result<Self, crate::error::AppError> {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&path).map_err(|error| {
            crate::error::AppError::Internal(format!(
                "could not create self-update temporary directory {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[derive(serde::Deserialize)]
struct ReleaseManifest {
    format_version: u32,
    version: String,
    target: String,
    revision: String,
}

pub fn run(source: crate::app::UpdateSource) -> Result<SelfUpdateReport, crate::error::AppError> {
    let bundle = prepare_bundle(source)?;
    let cli = update_cli(&bundle);
    let codex = update_skill(&bundle, "bettr", codex_skill_destination());
    let claude = update_skill(&bundle, "bettr-claude", claude_skill_destination());
    Ok(SelfUpdateReport {
        source,
        revision: bundle.revision.clone(),
        cli,
        codex,
        claude,
    })
}

pub fn uninstall() -> Result<SelfUninstallReport, crate::error::AppError> {
    Ok(SelfUninstallReport {
        codex: uninstall_skill("bettr", codex_skill_destination()),
        claude: uninstall_skill("bettr-claude", claude_skill_destination()),
    })
}

fn prepare_bundle(source: crate::app::UpdateSource) -> Result<Bundle, crate::error::AppError> {
    let temporary_directory = TemporaryDirectory::new("bettr-self-update")?;
    match source {
        crate::app::UpdateSource::Release => prepare_release_bundle(temporary_directory),
        crate::app::UpdateSource::Main => prepare_main_bundle(temporary_directory),
    }
}

fn prepare_release_bundle(
    temporary_directory: TemporaryDirectory,
) -> Result<Bundle, crate::error::AppError> {
    let release = parse_json_object(&curl_text(RELEASE_API_URL)?, "latest release")?;
    let tag = json_string(&release, "tag_name", "latest release")?;
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_owned();
    if version.is_empty() {
        return Err(crate::error::AppError::Internal(
            "latest release has an empty version".to_owned(),
        ));
    }
    let target = target_triple()?;
    let archive_name = format!("bettr-{version}-{target}.tar.gz");
    let checksum_name = format!("{archive_name}.sha256");
    let archive_url = release_asset_url(&release, &archive_name)?;
    let checksum_url = release_asset_url(&release, &checksum_name)?;
    let archive_path = temporary_directory.path.join(&archive_name);
    let checksum_path = temporary_directory.path.join(&checksum_name);
    curl_download(&archive_url, &archive_path)?;
    curl_download(&checksum_url, &checksum_path)?;
    verify_checksum(&archive_path, &checksum_path, &archive_name)?;

    let root = temporary_directory.path.join("release");
    std::fs::create_dir(&root).map_err(|error| {
        crate::error::AppError::Internal(format!(
            "could not create release extraction directory: {error}"
        ))
    })?;
    validate_archive_paths(&archive_path)?;
    extract_archive(&archive_path, &root)?;
    let manifest = read_release_manifest(&root)?;
    if manifest.format_version != 1 {
        return Err(crate::error::AppError::InvalidInput(format!(
            "unsupported self-update manifest version {}",
            manifest.format_version
        )));
    }
    if manifest.version != version || manifest.target != target || manifest.revision.is_empty() {
        return Err(crate::error::AppError::InvalidInput(
            "release manifest does not match the downloaded release".to_owned(),
        ));
    }
    let binary = root.join("bettr");
    validate_bundle_root(&root, &binary)?;
    if binary_version(&binary)? != manifest.version {
        return Err(crate::error::AppError::InvalidInput(
            "release manifest does not match the bundled CLI version".to_owned(),
        ));
    }
    Ok(Bundle {
        _temporary_directory: temporary_directory,
        root,
        binary,
        source: crate::app::UpdateSource::Release,
        version: manifest.version,
        revision: manifest.revision,
    })
}

fn prepare_main_bundle(
    temporary_directory: TemporaryDirectory,
) -> Result<Bundle, crate::error::AppError> {
    let commit = parse_json_object(&curl_text(MAIN_API_URL)?, "main commit")?;
    let revision = json_string(&commit, "sha", "main commit")?;
    let archive_url = format!("https://github.com/{REPOSITORY}/archive/{revision}.tar.gz");
    let archive_path = temporary_directory.path.join("main.tar.gz");
    curl_download(&archive_url, &archive_path)?;
    validate_archive_paths(&archive_path)?;
    let extracted = temporary_directory.path.join("main");
    std::fs::create_dir(&extracted).map_err(|error| {
        crate::error::AppError::Internal(format!(
            "could not create main extraction directory: {error}"
        ))
    })?;
    extract_archive(&archive_path, &extracted)?;
    let source_root = single_directory(&extracted)?;
    let binary = source_root.join("target/release/bettr");
    let mut command = std::process::Command::new("cargo");
    command.args(["build", "--locked", "--release"]);
    command.current_dir(&source_root);
    run_command(&mut command, "cargo build")?;
    let version = binary_version(&binary)?;
    validate_bundle_root(&source_root, &binary)?;
    Ok(Bundle {
        _temporary_directory: temporary_directory,
        root: source_root,
        binary,
        source: crate::app::UpdateSource::Main,
        version,
        revision,
    })
}

fn update_cli(bundle: &Bundle) -> ComponentUpdate {
    let path = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            return failed_component(
                bundle,
                "current executable".to_owned(),
                None,
                format!("could not resolve current executable: {error}"),
            );
        }
    };
    let display_path = path.display().to_string();
    let staged = sibling_temp_path(&path, "cli");
    let result = (|| {
        std::fs::copy(&bundle.binary, &staged).map_err(|error| {
            crate::error::AppError::Internal(format!(
                "could not stage CLI update {}: {error}",
                bundle.binary.display()
            ))
        })?;
        set_executable(&staged)?;
        std::fs::rename(&staged, &path).map_err(|error| {
            crate::error::AppError::Internal(format!(
                "could not replace CLI at {display_path}: {error}"
            ))
        })
    })();
    match result {
        Ok(()) => component(bundle, display_path, UpdateResult::Updated, None, None),
        Err(error) => {
            let _ = remove_path(&staged);
            failed_component(bundle, display_path, None, error.to_string())
        }
    }
}

fn update_skill(
    bundle: &Bundle,
    source_name: &str,
    destination: Result<std::path::PathBuf, crate::error::AppError>,
) -> ComponentUpdate {
    let destination = match destination {
        Ok(destination) => destination,
        Err(error) => {
            return failed_component(bundle, source_name.to_owned(), None, error.to_string());
        }
    };
    let display_path = destination.display().to_string();
    let backup = backup_path(&destination).ok();
    match install_skill(&bundle.root.join("skills").join(source_name), &destination) {
        Ok(result) => component(bundle, display_path, result, None, None),
        Err(error) => failed_component(
            bundle,
            display_path,
            backup.filter(|path| path_exists(path)),
            error.to_string(),
        ),
    }
}

fn uninstall_skill(
    source_name: &str,
    destination: Result<std::path::PathBuf, crate::error::AppError>,
) -> ComponentUninstall {
    let destination = match destination {
        Ok(destination) => destination,
        Err(error) => {
            return ComponentUninstall {
                result: UninstallResult::Failed,
                path: source_name.to_owned(),
                error: Some(error.to_string()),
            };
        }
    };
    let path = destination.display().to_string();
    match remove_installed_skill(&destination, source_name) {
        Ok(result) => ComponentUninstall {
            result,
            path,
            error: None,
        },
        Err(error) => ComponentUninstall {
            result: UninstallResult::Failed,
            path,
            error: Some(error.to_string()),
        },
    }
}

fn component(
    bundle: &Bundle,
    path: String,
    result: UpdateResult,
    backup: Option<String>,
    error: Option<String>,
) -> ComponentUpdate {
    ComponentUpdate {
        source: bundle.source,
        version: Some(bundle.version.clone()),
        revision: bundle.revision.clone(),
        result,
        path,
        backup,
        error,
    }
}

fn failed_component(
    bundle: &Bundle,
    path: String,
    backup: Option<std::path::PathBuf>,
    error: String,
) -> ComponentUpdate {
    component(
        bundle,
        path,
        UpdateResult::Failed,
        backup.map(|path| path.display().to_string()),
        Some(error),
    )
}

fn install_skill(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<UpdateResult, crate::error::AppError> {
    if !source.is_dir() {
        return Err(crate::error::AppError::Internal(format!(
            "skill source is missing: {}",
            source.display()
        )));
    }
    let parent = destination.parent().ok_or_else(|| {
        crate::error::AppError::InvalidInput(format!(
            "skill destination has no parent: {}",
            destination.display()
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        crate::error::AppError::Internal(format!(
            "could not create skill directory {}: {error}",
            parent.display()
        ))
    })?;
    let staged = sibling_temp_path(destination, "skill");
    if let Err(error) = copy_tree(source, &staged) {
        let _ = remove_path(&staged);
        return Err(error);
    }

    let existed = path_exists(destination);
    let backup = backup_path(destination)?;
    if existed {
        if path_exists(&backup) {
            let _ = remove_path(&staged);
            return Err(crate::error::AppError::Conflict(format!(
                "skill backup already exists: {}",
                backup.display()
            )));
        }
        if let Err(error) = std::fs::rename(destination, &backup) {
            let _ = remove_path(&staged);
            return Err(crate::error::AppError::Internal(format!(
                "could not back up skill {} to {}: {error}",
                destination.display(),
                backup.display()
            )));
        }
    }

    if let Err(error) = std::fs::rename(&staged, destination) {
        let _ = remove_path(&staged);
        return Err(crate::error::AppError::Internal(format!(
            "could not install skill {}; backup retained at {}: {error}",
            destination.display(),
            backup.display()
        )));
    }
    if existed {
        remove_path(&backup).map_err(|error| {
            crate::error::AppError::Internal(format!(
                "skill updated but backup could not be removed at {}: {error}",
                backup.display()
            ))
        })?;
        Ok(UpdateResult::Updated)
    } else {
        Ok(UpdateResult::Installed)
    }
}

fn remove_installed_skill(
    destination: &std::path::Path,
    expected_name: &str,
) -> Result<UninstallResult, crate::error::AppError> {
    let metadata = match std::fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UninstallResult::NotInstalled);
        }
        Err(error) => {
            return Err(crate::error::AppError::Internal(format!(
                "could not inspect skill destination {}: {error}",
                destination.display()
            )));
        }
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(crate::error::AppError::Conflict(format!(
            "refusing to remove symlink skill destination: {}",
            destination.display()
        )));
    }
    if !file_type.is_dir() {
        return Err(crate::error::AppError::Conflict(format!(
            "refusing to remove non-directory skill destination: {}",
            destination.display()
        )));
    }

    let manifest = destination.join("SKILL.md");
    let contents = std::fs::read_to_string(&manifest).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            crate::error::AppError::Conflict(format!(
                "refusing to remove unmanaged skill directory: {}",
                destination.display()
            ))
        } else {
            crate::error::AppError::Internal(format!(
                "could not read skill manifest {}: {error}",
                manifest.display()
            ))
        }
    })?;
    if !has_skill_name(&contents, expected_name) {
        return Err(crate::error::AppError::Conflict(format!(
            "refusing to remove unmanaged skill directory: {}",
            destination.display()
        )));
    }

    remove_path(destination).map_err(|error| {
        crate::error::AppError::Internal(format!(
            "could not remove skill directory {}: {error}",
            destination.display()
        ))
    })?;
    Ok(UninstallResult::Removed)
}

fn has_skill_name(contents: &str, expected_name: &str) -> bool {
    let mut lines = contents.lines();
    if lines.next() != Some("---") {
        return false;
    }
    for line in lines {
        if line == "---" {
            break;
        }
        if line.trim() == format!("name: {expected_name}") {
            return true;
        }
    }
    false
}

fn copy_tree(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), crate::error::AppError> {
    std::fs::create_dir_all(destination).map_err(|error| {
        crate::error::AppError::Internal(format!(
            "could not create staged skill {}: {error}",
            destination.display()
        ))
    })?;
    for entry in std::fs::read_dir(source).map_err(|error| {
        crate::error::AppError::Internal(format!(
            "could not read skill source {}: {error}",
            source.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            crate::error::AppError::Internal(format!("could not read skill entry: {error}"))
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| {
            crate::error::AppError::Internal(format!(
                "could not inspect skill entry {}: {error}",
                source_path.display()
            ))
        })?;
        if file_type.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &destination_path).map_err(|error| {
                crate::error::AppError::Internal(format!(
                    "could not copy skill file {}: {error}",
                    source_path.display()
                ))
            })?;
        } else {
            return Err(crate::error::AppError::InvalidInput(format!(
                "unsupported skill entry: {}",
                source_path.display()
            )));
        }
    }
    Ok(())
}

fn backup_path(
    destination: &std::path::Path,
) -> Result<std::path::PathBuf, crate::error::AppError> {
    let name = destination
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| {
            crate::error::AppError::InvalidInput(format!(
                "skill destination must have a UTF-8 name: {}",
                destination.display()
            ))
        })?;
    Ok(destination
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!(".{name}.bettr-self-update-backup")))
}

fn sibling_temp_path(path: &std::path::Path, kind: &str) -> std::path::PathBuf {
    let name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("bettr");
    path.parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!(
            ".{name}.bettr-self-update-{kind}-{}",
            uuid::Uuid::new_v4().simple()
        ))
}

fn path_exists(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn remove_path(path: &std::path::Path) -> Result<(), std::io::Error> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn codex_skill_destination() -> Result<std::path::PathBuf, crate::error::AppError> {
    skill_destination("CODEX_HOME", ".codex", "bettr")
}

fn claude_skill_destination() -> Result<std::path::PathBuf, crate::error::AppError> {
    skill_destination("CLAUDE_CONFIG_DIR", ".claude", "bettr-claude")
}

fn skill_destination(
    environment_name: &str,
    default_directory: &str,
    skill_name: &str,
) -> Result<std::path::PathBuf, crate::error::AppError> {
    let root = match std::env::var_os(environment_name) {
        Some(path) if !path.is_empty() => std::path::PathBuf::from(path),
        Some(_) => {
            return Err(crate::error::AppError::InvalidInput(format!(
                "{environment_name} must not be empty"
            )));
        }
        None => std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|home| home.join(default_directory))
            .ok_or_else(|| {
                crate::error::AppError::InvalidInput(format!(
                    "{environment_name} or HOME must be set"
                ))
            })?,
    };
    Ok(root.join("skills").join(skill_name))
}

fn target_triple() -> Result<String, crate::error::AppError> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => Ok("x86_64-unknown-linux-gnu".to_owned()),
        ("aarch64", "linux") => Ok("aarch64-unknown-linux-gnu".to_owned()),
        ("x86_64", "macos") => Ok("x86_64-apple-darwin".to_owned()),
        ("aarch64", "macos") => Ok("aarch64-apple-darwin".to_owned()),
        (architecture, operating_system) => Err(crate::error::AppError::InvalidInput(format!(
            "self-update does not support {architecture}-{operating_system}"
        ))),
    }
}

fn curl_text(url: &str) -> Result<String, crate::error::AppError> {
    let mut command = std::process::Command::new("curl");
    command.args(["-fsSL", "--retry", "3", url]);
    let output = run_command(&mut command, "curl")?;
    String::from_utf8(output.stdout).map_err(|error| {
        crate::error::AppError::Internal(format!("curl returned non-UTF-8 data: {error}"))
    })
}

fn curl_download(url: &str, destination: &std::path::Path) -> Result<(), crate::error::AppError> {
    let mut command = std::process::Command::new("curl");
    command
        .args(["-fsSL", "--retry", "3", "--output"])
        .arg(destination)
        .arg(url);
    run_command(&mut command, "curl").map(|_| ())
}

fn run_command(
    command: &mut std::process::Command,
    name: &str,
) -> Result<std::process::Output, crate::error::AppError> {
    let output = command.output().map_err(|error| {
        crate::error::AppError::Internal(format!("could not run {name}: {error}"))
    })?;
    if output.status.success() {
        Ok(output)
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        Err(crate::error::AppError::Internal(format!(
            "{name} failed with status {}{suffix}",
            output.status
        )))
    }
}

fn parse_json_object(
    contents: &str,
    description: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, crate::error::AppError> {
    let value = serde_json::from_str::<serde_json::Value>(contents).map_err(|error| {
        crate::error::AppError::Internal(format!("could not parse {description} response: {error}"))
    })?;
    match value {
        serde_json::Value::Object(object) => Ok(object),
        _ => Err(crate::error::AppError::Internal(format!(
            "{description} response is not a JSON object"
        ))),
    }
}

fn json_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    description: &str,
) -> Result<String, crate::error::AppError> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            crate::error::AppError::Internal(format!("{description} response is missing {key}"))
        })
}

fn release_asset_url(
    release: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<String, crate::error::AppError> {
    release
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .and_then(|assets| {
            assets.iter().find_map(|asset| {
                (asset.get("name").and_then(serde_json::Value::as_str) == Some(name))
                    .then(|| asset.get("browser_download_url"))
                    .flatten()
                    .and_then(serde_json::Value::as_str)
            })
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            crate::error::AppError::NotFound(format!(
                "release asset {name} is not available for this platform"
            ))
        })
}

fn verify_checksum(
    archive: &std::path::Path,
    checksum: &std::path::Path,
    archive_name: &str,
) -> Result<(), crate::error::AppError> {
    let contents = std::fs::read_to_string(checksum).map_err(|error| {
        crate::error::AppError::Internal(format!(
            "could not read checksum {}: {error}",
            checksum.display()
        ))
    })?;
    let mut fields = contents.split_whitespace();
    let expected = fields
        .next()
        .ok_or_else(|| crate::error::AppError::InvalidInput("checksum file is empty".to_owned()))?;
    let listed_name = fields.next().map(|name| name.trim_start_matches('*'));
    if listed_name != Some(archive_name) {
        return Err(crate::error::AppError::InvalidInput(
            "checksum file names a different archive".to_owned(),
        ));
    }
    let bytes = std::fs::read(archive).map_err(|error| {
        crate::error::AppError::Internal(format!(
            "could not read release archive {}: {error}",
            archive.display()
        ))
    })?;
    use sha2::Digest as _;
    let actual = format!("{:x}", sha2::Sha256::digest(bytes));
    if actual != expected {
        return Err(crate::error::AppError::InvalidInput(
            "release archive checksum does not match".to_owned(),
        ));
    }
    Ok(())
}

fn validate_archive_paths(archive: &std::path::Path) -> Result<(), crate::error::AppError> {
    let mut command = std::process::Command::new("tar");
    command.args(["-tzf"]).arg(archive);
    let output = run_command(&mut command, "tar")?;
    let listing = String::from_utf8_lossy(&output.stdout);
    for entry in listing.lines() {
        let path = std::path::Path::new(entry.trim_end_matches('/'));
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(crate::error::AppError::InvalidInput(format!(
                "release archive contains an unsafe path: {entry}"
            )));
        }
    }

    let mut command = std::process::Command::new("tar");
    command.args(["-tvzf"]).arg(archive);
    let output = run_command(&mut command, "tar")?;
    for entry in String::from_utf8_lossy(&output.stdout).lines() {
        if !matches!(entry.as_bytes().first(), Some(b'-' | b'd')) {
            return Err(crate::error::AppError::InvalidInput(
                "release archive contains a non-regular entry".to_owned(),
            ));
        }
    }
    Ok(())
}

fn extract_archive(
    archive: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), crate::error::AppError> {
    let mut command = std::process::Command::new("tar");
    command
        .args(["-xzf"])
        .arg(archive)
        .args(["-C"])
        .arg(destination);
    run_command(&mut command, "tar").map(|_| ())
}

fn read_release_manifest(
    root: &std::path::Path,
) -> Result<ReleaseManifest, crate::error::AppError> {
    let path = root.join("manifest.json");
    let contents = std::fs::read_to_string(&path).map_err(|error| {
        crate::error::AppError::InvalidInput(format!(
            "release archive is missing manifest.json: {error}"
        ))
    })?;
    serde_json::from_str(&contents).map_err(|error| {
        crate::error::AppError::InvalidInput(format!("release manifest is invalid: {error}"))
    })
}

fn validate_bundle_root(
    root: &std::path::Path,
    binary: &std::path::Path,
) -> Result<(), crate::error::AppError> {
    for path in [
        binary.to_owned(),
        root.join("skills/bettr/SKILL.md"),
        root.join("skills/bettr-claude/SKILL.md"),
    ] {
        if !path.is_file() {
            return Err(crate::error::AppError::InvalidInput(format!(
                "self-update bundle is missing {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn single_directory(root: &std::path::Path) -> Result<std::path::PathBuf, crate::error::AppError> {
    let directories = std::fs::read_dir(root)
        .map_err(|error| {
            crate::error::AppError::Internal(format!("could not read source archive: {error}"))
        })?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(std::fs::FileType::is_dir)
                .map(|_| entry.path())
        })
        .collect::<Vec<_>>();
    if directories.len() != 1 {
        return Err(crate::error::AppError::InvalidInput(
            "source archive must contain one root directory".to_owned(),
        ));
    }
    Ok(directories.into_iter().next().expect("length checked"))
}

fn binary_version(binary: &std::path::Path) -> Result<String, crate::error::AppError> {
    let mut command = std::process::Command::new(binary);
    command.arg("--version");
    let output = run_command(&mut command, "updated bettr --version")?;
    let version = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_owned);
    version.ok_or_else(|| {
        crate::error::AppError::InvalidInput(
            "updated bettr binary returned an invalid version".to_owned(),
        )
    })
}

fn set_executable(path: &std::path::Path) -> Result<(), crate::error::AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(path)
            .map_err(|error| {
                crate::error::AppError::Internal(format!(
                    "could not inspect staged CLI {}: {error}",
                    path.display()
                ))
            })?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).map_err(|error| {
            crate::error::AppError::Internal(format!(
                "could not make staged CLI executable: {error}"
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn installs_a_missing_skill() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("codex/skills/bettr");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "new skill").unwrap();

        let result = super::install_skill(&source, &destination).unwrap();

        assert_eq!(result, super::UpdateResult::Installed);
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "new skill"
        );
    }

    #[test]
    fn backs_up_and_removes_the_backup_after_a_successful_skill_update() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("codex/skills/bettr");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(source.join("SKILL.md"), "new skill").unwrap();
        std::fs::write(destination.join("SKILL.md"), "old skill").unwrap();

        let backup = super::backup_path(&destination).unwrap();
        let result = super::install_skill(&source, &destination).unwrap();

        assert_eq!(result, super::UpdateResult::Updated);
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "new skill"
        );
        assert!(!backup.exists());
    }

    #[test]
    fn refuses_to_overwrite_an_existing_recovery_backup() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("codex/skills/bettr");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(source.join("SKILL.md"), "new skill").unwrap();
        std::fs::write(destination.join("SKILL.md"), "old skill").unwrap();
        let backup = super::backup_path(&destination).unwrap();
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(backup.join("SKILL.md"), "recovery").unwrap();

        let error = super::install_skill(&source, &destination).unwrap_err();

        assert!(error.to_string().contains("backup already exists"));
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "old skill"
        );
        assert_eq!(
            std::fs::read_to_string(backup.join("SKILL.md")).unwrap(),
            "recovery"
        );
    }

    #[test]
    fn uninstalls_a_managed_skill() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("codex/skills/bettr");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(
            destination.join("SKILL.md"),
            "---\nname: bettr\n---\n# Bettr\n",
        )
        .unwrap();
        std::fs::write(destination.join("README.md"), "managed").unwrap();

        let result = super::remove_installed_skill(&destination, "bettr").unwrap();

        assert_eq!(result, super::UninstallResult::Removed);
        assert!(!destination.exists());
        assert!(destination.parent().unwrap().exists());
    }

    #[test]
    fn reports_a_missing_skill_without_changing_its_parent() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("codex/skills/bettr");
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();

        let result = super::remove_installed_skill(&destination, "bettr").unwrap();

        assert_eq!(result, super::UninstallResult::NotInstalled);
        assert!(destination.parent().unwrap().exists());
    }

    #[test]
    fn refuses_to_remove_an_unmanaged_skill_directory() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("codex/skills/bettr");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("SKILL.md"), "user skill").unwrap();

        let error = super::remove_installed_skill(&destination, "bettr").unwrap_err();

        assert!(error.to_string().contains("unmanaged skill directory"));
        assert!(destination.exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_remove_a_symlink_skill_destination() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let destination = directory.path().join("codex/skills/bettr");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &destination).unwrap();

        let error = super::remove_installed_skill(&destination, "bettr").unwrap_err();

        assert!(error.to_string().contains("refusing to remove symlink"));
        assert!(destination.exists());
        assert!(target.exists());
    }
}
