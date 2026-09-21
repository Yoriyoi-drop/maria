#!/usr/bin/env python3
"""
Mivon Auto-Patch Detector
Detects new patches and updates within the Mivon project itself.
Runs automatically to check for updates and trigger GitHub Actions.
"""

import json
import os
import sys
import hashlib
from pathlib import Path
from datetime import datetime, timezone
from typing import Optional, Dict, List, Tuple

# Configuration
PROJECT_ROOT = Path(__file__).resolve().parent.parent
STATE_DIR = PROJECT_ROOT / ".mivon" / "auto-update"
STATE_FILE = STATE_DIR / "patch-state.json"
VERSION_FILE = PROJECT_ROOT / "Cargo.toml"
LOG_FILE = STATE_DIR / "detector.log"
REPO_URL = "https://api.github.com/repos/mivonsim/mivon"


def log(message: str, level: str = "INFO") -> None:
    """Log message with timestamp"""
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    timestamp = datetime.now(timezone.utc).isoformat()
    with open(LOG_FILE, "a") as f:
        f.write(f"[{timestamp}] [{level}] {message}\n")
    print(f"[{level}] {message}")


def get_current_version() -> str:
    """Extract current version from Cargo.toml"""
    try:
        content = VERSION_FILE.read_text()
        for line in content.splitlines():
            if line.strip().startswith("version ="):
                return line.split('"')[1]
    except Exception as e:
        log(f"Failed to read version: {e}", "ERROR")
    return "0.0.0"


def get_file_hash(filepath: Path) -> Optional[str]:
    """Get SHA256 hash of a file"""
    try:
        if not filepath.exists():
            return None
        return hashlib.sha256(filepath.read_bytes()).hexdigest()
    except Exception as e:
        log(f"Failed to hash {filepath}: {e}", "ERROR")
        return None


def get_source_files() -> List[Path]:
    """Get all source files that affect Mivon behavior"""
    patterns = [
        "src/**/*.rs",
        "crates/**/*.rs",
        "Cargo.toml",
        "Cargo.lock",
        "install.sh",
        ".github/workflows/*.yml",
    ]
    
    files = []
    for pattern in patterns:
        files.extend(PROJECT_ROOT.glob(pattern))
    
    return sorted([f for f in files if f.is_file()])


def get_current_state() -> Dict:
    """Get current patch state (file hashes)"""
    state = {}
    for filepath in get_source_files():
        rel_path = str(filepath.relative_to(PROJECT_ROOT))
        file_hash = get_file_hash(filepath)
        if file_hash:
            state[rel_path] = file_hash
    
    return state


def load_previous_state() -> Dict:
    """Load previous patch state"""
    if not STATE_FILE.exists():
        return {}
    
    try:
        return json.loads(STATE_FILE.read_text())
    except Exception as e:
        log(f"Failed to load previous state: {e}", "ERROR")
        return {}


def save_state(state: Dict) -> None:
    """Save current patch state"""
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    STATE_FILE.write_text(json.dumps(state, indent=2))


def detect_changes(old_state: Dict, new_state: Dict) -> List[str]:
    """Detect changed files between states"""
    changes = []
    
    for filepath, new_hash in new_state.items():
        old_hash = old_state.get(filepath)
        if old_hash is None:
            changes.append(f"ADDED: {filepath}")
        elif old_hash != new_hash:
            changes.append(f"MODIFIED: {filepath}")
    
    for filepath in old_state:
        if filepath not in new_state:
            changes.append(f"REMOVED: {filepath}")
    
    return changes


def check_for_updates() -> Tuple[bool, List[str]]:
    """Check if there are updates available"""
    old_state = load_previous_state()
    new_state = get_current_state()
    
    changes = detect_changes(old_state, new_state)
    
    if changes:
        log(f"Detected {len(changes)} file changes")
        for change in changes:
            log(f"  {change}")
        
        save_state(new_state)
        return True, changes
    
    log("No changes detected")
    return False, []


def trigger_github_action(changes: List[str]) -> bool:
    """Prepare payload dan cetak perintah untuk user (GATE: tidak pernah
    auto-dispatch publish). Opsi --dispatch-prep hanya memanggil release-prep.yml
    (aman: membuat DRAFT saja, tidak publish/commit apa pun)."""
    try:
        # Create payload
        payload = {
            "event_type": "mivon-auto-update",
            "client_payload": {
                "version": get_current_version(),
                "changes": changes,
                "timestamp": datetime.now(timezone.utc).isoformat(),
                "source": "mivon-detector"
            }
        }

        # Write payload for GitHub Actions to consume
        payload_file = STATE_DIR / "update-payload.json"
        payload_file.write_text(json.dumps(payload, indent=2))

        log(f"Update payload written to {payload_file}")

        # Desain final MIVON: RILIS HANYA VIA TAG VERSI (gerbang ketat).
        # Detector TIDAK PERNAH men-dispatch publish otomatis — hanya
        # menyiapkan payload + mencetak langkah yang harus dilakukan user.
        log("Rilis Mivon wajib lewat TAG versi:")
        log(f"  1. bump versi di Cargo.toml (saat ini v{get_current_version()}), commit, push")
        log("  2. git tag v<versi> && git push origin v<versi>")
        log("  3. workflow release.yml otomatis berjalan — CI hijau wajib,")
        log("     artefak diverifikasi, release + landing + manifest sinkron.")

        return True

    except Exception as e:
        log(f"Failed to trigger GitHub Action: {e}", "ERROR")
        return False


def should_auto_update(changes: List[str]) -> bool:
    """Determine if changes warrant automatic update"""
    critical_paths = [
        "src/main.rs",
        "src/cli.rs",
        "Cargo.toml",
        "Cargo.lock",
        "install.sh",
        ".github/workflows/",
    ]
    
    for change in changes:
        for path in critical_paths:
            if path in change:
                return True
    
    return False


def main() -> int:
    """Main detector function"""
    log("Mivon Auto-Patch Detector starting...")
    
    current_version = get_current_version()
    log(f"Current version: {current_version}")
    
    has_updates, changes = check_for_updates()
    
    if not has_updates:
        log("No updates detected, exiting")
        return 0
    
    if should_auto_update(changes):
        log("Perubahan kritikal terdeteksi — update TIDAK dieksekusi otomatis.")
        log("Seleksi ketat aktif: menunggu perintah user untuk mulai.")
        trigger_github_action(changes)
        return 0
    else:
        log("Perubahan non-kritikal, tidak menandai auto-update")
        return 0


if __name__ == "__main__":
    sys.exit(main())