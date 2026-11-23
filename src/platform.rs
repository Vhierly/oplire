//! Platform-specific WARP helpers.
//!
//! Upstream `oplire` only implements the Linux reset path (`systemctl` +
//! `/var/lib/cloudflare-warp`). On Windows the WARP client keeps its
//! registration inside the service, and `warp-cli registration new` refuses to
//! run while the old registration still exists. The Windows sequence is
//! therefore: `disconnect` -> `registration delete` -> `registration new` ->
//! `connect` (plus a DNS flush).
//!
//! Binaries shipped inside an MSIX/Store package (for example the Hermes Agent
//! payload under `C:\Program Files\WindowsApps`) cannot be spawned by a normal
//! process, so `warp-cli` is resolved by absolute path instead of relying on a
//! `PATH` entry that may point there.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn log(verbose: bool, msg: &str) {
    if verbose {
        eprintln!("  [warp] {}", msg);
    }
}

/// `warp-cli` executable: absolute path on Windows, `PATH` lookup elsewhere.
pub fn warp_cli() -> String {
    if cfg!(target_os = "windows") {
        resolve_windows_warp_cli()
    } else {
        "warp-cli".to_string()
    }
}

#[cfg(target_os = "windows")]
fn resolve_windows_warp_cli() -> String {
    let mut candidates: Vec<String> = Vec::new();
    for key in [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
    ] {
        if let Ok(dir) = std::env::var(key) {
            if !dir.is_empty() {
                candidates.push(format!(
                    r"{}\Cloudflare\Cloudflare WARP\warp-cli.exe",
                    dir.trim_end_matches('\\')
                ));
            }
        }
    }
    if let Ok(dir) = std::env::var("LOCALAPPDATA") {
        if !dir.is_empty() {
            candidates.push(format!(
                r"{}\Cloudflare\Cloudflare WARP\warp-cli.exe",
                dir.trim_end_matches('\\')
            ));
        }
    }
    candidates.push(r"C:\Program Files\Cloudflare\Cloudflare WARP\warp-cli.exe".to_string());
    candidates.push(
        r"C:\Program Files (x86)\Cloudflare\Cloudflare WARP\warp-cli.exe".to_string(),
    );

    for candidate in candidates {
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    "warp-cli".to_string()
}

#[cfg(not(target_os = "windows"))]
fn resolve_windows_warp_cli() -> String {
    "warp-cli".to_string()
}

/// True when `warp-cli --version` runs.
pub fn warp_installed() -> bool {
    Command::new(warp_cli())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run(args: &[&str]) -> Result<String, String> {
    let output = Command::new(warp_cli())
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            Err(format!("warp-cli {:?} failed", args))
        } else {
            Err(err)
        }
    }
}

/// Run `warp-cli <args>` and return its stdout.
pub fn run_warp(args: &[&str]) -> Result<String, String> {
    run(args)
}

fn run_ignore(args: &[&str]) {
    let _ = run(args);
}

fn flush_dns() {
    let _ = Command::new("ipconfig").arg("/flushdns").output();
}

/// Full reset: drop the old registration, register a new one, reconnect.
pub fn reset_full(verbose: bool) -> Result<(), String> {
    log(verbose, "disconnect");
    run_ignore(&["disconnect"]);
    sleep(1000);

    reset_registration(verbose)?;
    sleep(500);

    log(verbose, "connect");
    let _ = run(&["connect"])?;

    #[cfg(target_os = "windows")]
    {
        log(verbose, "flush DNS");
        flush_dns();
    }

    Ok(())
}

/// Fast rotation without restarting the service.
pub fn reset_quick(verbose: bool) -> Result<(), String> {
    log(verbose, "disconnect");
    run_ignore(&["disconnect"]);
    sleep(800);

    reset_registration(verbose)?;
    sleep(500);

    log(verbose, "connect");
    let _ = run(&["connect"])?;

    #[cfg(target_os = "windows")]
    {
        flush_dns();
    }

    Ok(())
}

/// Stop the tunnel (and the service on Linux).
pub fn stop(verbose: bool) -> Result<(), String> {
    log(verbose, "disconnect");
    run_ignore(&["disconnect"]);
    sleep(500);
    stop_service(verbose);
    Ok(())
}

#[cfg(target_os = "windows")]
fn reset_registration(verbose: bool) -> Result<(), String> {
    log(verbose, "registration delete");
    run_ignore(&["registration", "delete"]);
    sleep(500);

    log(verbose, "registration new");
    run(&["registration", "new"])?;
    sleep(1000);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn reset_registration(verbose: bool) -> Result<(), String> {
    log(verbose, "stop warp-svc");
    let _ = Command::new("sudo").args(["-n", "systemctl", "stop", "warp-svc"]).output();
    sleep(500);

    log(verbose, "clear cache");
    let _ = Command::new("sudo")
        .args(["-n", "sh", "-c", "rm -rf /var/lib/cloudflare-warp/*"])
        .output();
    sleep(500);

    log(verbose, "start warp-svc");
    let _ = Command::new("sudo").args(["-n", "systemctl", "start", "warp-svc"]).output();
    sleep(2000);

    log(verbose, "registration new");
    run(&["registration", "new"])?;
    sleep(1000);
    Ok(())
}

#[cfg(target_os = "windows")]
fn stop_service(_verbose: bool) {}

#[cfg(not(target_os = "windows"))]
fn stop_service(verbose: bool) {
    log(verbose, "stop warp-svc");
    let _ = Command::new("sudo").args(["-n", "systemctl", "stop", "warp-svc"]).output();
    log(verbose, "disable warp-svc");
    let _ = Command::new("sudo")
        .args(["-n", "systemctl", "disable", "warp-svc"])
        .output();
}

/// Human-readable `warp-cli status` output.
pub fn status_text() -> Result<String, String> {
    run(&["status"])
}

/// Download and run the Windows WARP installer (needs elevation).
#[cfg(target_os = "windows")]
pub fn install_warp_windows(verbose: bool) -> Result<(), String> {
    const INSTALLER: &str =
        "https://1111-releases.cloudflare.com/latest/Windows%20Cloudflare%20WARP%20Setup.exe";

    let dest = std::env::temp_dir().join("Cloudflare-WARP-Setup.exe");
    log(verbose, &format!("downloading {}", INSTALLER));

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = client
        .get(INSTALLER)
        .send()
        .map_err(|e| e.to_string())?
        .bytes()
        .map_err(|e| e.to_string())?;
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;

    log(verbose, "running installer");
    let status = Command::new(&dest)
        .args(["/quiet", "/install"])
        .status()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&dest);

    if status.success() {
        Ok(())
    } else {
        Err(format!("WARP installer exited with {}", status))
    }
}

#[cfg(not(target_os = "windows"))]
pub fn install_warp_windows(_verbose: bool) -> Result<(), String> {
    Err("Windows installer is only available on Windows".to_string())
}
