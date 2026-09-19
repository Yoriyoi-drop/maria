# GitHub Actions Management - Maria Project

This repository implements a sophisticated GitHub Actions workflow system for **automated release updates and landing page management** with strict validation.

## Overview

The workflow system ensures that:
- **Only authorized changes** are made to documentation
- **Manual triggers** are required for non-automatic updates
- **All changes** are validated before execution
- **Complete audit trail** is maintained for all updates

## Workflow Files

### 1. `release-update.yml` - Automated Release Updates

**Triggers:**
- Push to main branch with specific source paths (`src/**`, `Cargo.toml`, `Cargo.lock`, `crates/**`)
- Manual workflow dispatch with `update_landing` input

**Key Features:**
- **Path-based filtering** - Only updates when source code changes
- **Validation gate** - Strict selection check before proceeding
- **Automatic binary building** and verification
- **Documentation updates** in landing page
- **GitHub Release creation** with binaries
- **Comprehensive cleanup**

**Manual Trigger Usage:**

```bash
# Trigger manual landing page update
gh workflow run release-update.yml -f update_landing=true

# Re-run workflow manually
gh workflow run release-update.yml
```

### 2. `trigger-management.yml` - Manual Trigger Management

**Triggers:**
- Only manual workflow dispatch (no automatic execution)

**Inputs:**
- `trigger_type`: `manual`, `test`, or `update_landing`
- `force_update`: Boolean to bypass validation (for emergency updates)

**Key Features:**
- **Explicit permission required** for all operations
- **Three-tier validation** (normal, force, test)
- **Separate update flows** for landing page vs. internal updates
- **Complete audit logging** for all actions
- **Status reporting** for workflow execution

**Manual Trigger Usage:**

```bash
# Normal manual update
gh workflow run trigger-management.yml -f trigger_type=manual

# Force update (bypasses validation)
gh workflow run trigger-management.yml -f trigger_type=manual -f force_update=true

# Test environment update
gh workflow run trigger-management.yml -f trigger_type=test

# Landing page specific update
gh workflow run trigger-management.yml -f trigger_type=update_landing
```

## Update Process Flow

### Standard Automated Updates (`release-update.yml`)

1. **Path Validation** (push to main)
   - Check if changed files match source code paths
   - If not, workflow ends without action

2. **Build Pipeline**
   - Setup Rust toolchain
   - Cache cargo artifacts
   - Build release binary
   - Upload artifacts

3. **Documentation Updates**
   - Update installation.mdx in landing page
   - Add version-specific instructions
   - Commit and push to main

4. **Binary Verification**
   - Download built binaries
   - Verify functionality with `--help` command
   - Create update summary

5. **GitHub Release**
   - Create official release with tag v<version>
   - Include binary in release assets
   - Generate release notes

6. **Cleanup**
   - Remove temporary files
   - Finalize workflow

### Manual Updates (`trigger-management.yml`)

1. **Validation Phase**
   - Verify manual trigger type
   - Check force update permission
   - Approve or deny execution

2. **Artifact Preparation**
   - Build release binary
   - Upload for downstream jobs

3. **Targeted Updates**
   - **Landing Page**: Update README.md and install.sh
   - **Install Script**: Update with current workflow metadata

4. **Verification**
   - Test binary functionality
   - Confirm all documentation changes

5. **Release Creation** (manual only)
   - Create GitHub release with detailed metadata
   - Include workflow execution context

## Installation Updates

### Automatic Updates (Recommended)

The most common way to update Maria is through automatic release updates:

```bash
# Automatic installation from GitHub releases
curl -fsSL https://raw.githubusercontent.com/Yoriyoi-drop/maria/main/install.sh -o install.sh
sudo bash install.sh
```

### Build from Source

```bash
git clone https://github.com/Yoriyoi-drop/maria.git
cd maria
cargo build --release
```

### Update Documentation

For documentation updates:

```bash
# Trigger manual landing page update
gh workflow run release-update.yml -f update_landing=true
```

## Security Considerations

### Validation Rules

1. **Branch Protection**: All updates require main branch changes
2. **Path Filtering**: Only source code paths trigger automatic updates
3. **Explicit Manual Triggers**: All manual updates require workflow_dispatch
4. **Force Update Safety**: Force updates require explicit approval

### Access Control

- All workflows run with GitHub Actions bot account
- Secret tokens are managed through repository secrets
- Read-only access for most operations
- Write access only for committed changes

## Troubleshooting

### Common Issues

#### Workflow Not Triggering

```bash
# Check workflow file syntax
gh workflow list
```

#### Manual Trigger Errors

```bash
# Verify required inputs
gh workflow view trigger-management.yml --json inputs
```

#### Binary Verification Failures

```bash
# Check binary functionality
./artifacts/maria --help
```

### Debugging

```bash
# View workflow run logs
gh run list
# Get specific run logs
gh run view <run-id> --log-fuse
```

## Integration with CI/CD

### Local Testing

```bash
# Test local builds
cargo build --release
./target/release/maria --help
```

### Branch Management

```bash
# Create feature branch
git checkout -b feature/update-docs
# Make changes
# Update when ready
```

### Pull Request Process

1. Make changes to source code or documentation
2. Commit changes with appropriate commit message
3. Push to feature branch
4. Create pull request to main
5. Wait for CI/CD validation
6. Merge when all checks pass

## Files Modified

### During Workflow Execution

- `README.md`: Updated with latest release information
- `install.sh`: Updated with current workflow logic
- `.github/auto-update.log`: Internal workflow logging
- Various generated files in `artifacts/` directory

### During Manual Updates

- `README.md`: Enhanced with version-specific instructions
- `install.sh`: Enhanced with workflow metadata
- GitHub Releases: Created for each manual update

## Monitoring

### Workflow Status

Track workflow execution:

```bash
# List recent runs
gh run list --limit 10

# Check workflow status
gh api repos/Yoriyoi-drop/maria/actions/runs
```

### Update Notifications

Subscribe to workflow notifications:

```bash
# Enable workflow run notifications
gh api user/subscriptions -X PUT repos/Yoriyoi-drop/maria
```

## Best Practices

### For Contributors

1. **Always test locally** before making changes
2. **Use feature branches** for all modifications
3. **Follow conventional commits** for commit messages
4. **Document changes** in commit messages

### For Maintainers

1. **Review workflow changes** before merging
2. **Test manual triggers** before relying on them
3. **Monitor workflow logs** for issues
4. **Maintain documentation** for update processes

## FAQ

### How do I trigger an update?

Use GitHub CLI:
```bash
gh workflow run <workflow-name> -f <input>=<value>
```

### What are the required permissions?

Repository write access for most operations, admin access for emergency force updates.

### How do I check workflow status?

Use GitHub CLI:
```bash
gh run list
gh run view <run-id>
```

### Can I customize the update process?

Yes, modify the workflow YAML files in `.github/workflows/`.

### What happens if a workflow fails?

Check the workflow logs for error details and follow troubleshooting steps.

## Support

For issues with GitHub Actions:
1. Check workflow logs
2. Verify syntax and permissions
3. Test manually with `gh workflow run`
4. Review recent changes to workflow files

For installation issues:
1. Use the automatic install script
2. Build from source if needed
3. Check PATH and permissions
4. Verify binary functionality with `--help`