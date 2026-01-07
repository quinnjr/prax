#!/usr/bin/env bash
#
# Prax Workspace Publish Script
#
# Publishes all crates to crates.io in dependency order to avoid version conflicts.
# Automatically skips crates that are already published with the same version.
#
# Usage:
#   ./scripts/publish.sh [OPTIONS]
#
# Options:
#   --dry-run             Perform a dry run without actually publishing
#   --no-verify           Skip cargo verify step (not recommended)
#   --allow-dirty         Allow publishing with uncommitted changes
#   --skip-version-check  Skip checking if version is already on crates.io
#   --version VER         Set version for all crates before publishing
#   --help                Show this help message
#
# Requirements:
#   - cargo login must be configured with a valid crates.io token
#   - All tests must pass
#   - Working directory must be clean (unless --allow-dirty)
#   - curl must be available for crates.io API checks

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default options
DRY_RUN=false
NO_VERIFY=false
ALLOW_DIRTY=false
SKIP_VERSION_CHECK=false
NEW_VERSION=""
DELAY_SECONDS=30  # Delay between publishes to let crates.io index

# Crates in dependency order (topologically sorted)
# Tier 1: No internal dependencies
TIER_1=(
    "prax-schema"
    "prax-query"
)

# Tier 2: Depends only on Tier 1
TIER_2=(
    "prax-codegen"
    "prax-migrate"
    "prax-postgres"
    "prax-mysql"
    "prax-sqlite"
    "prax-sqlx"
)

# Tier 3: Depends on Tier 1 and/or Tier 2
TIER_3=(
    "prax-armature"
    "prax-axum"
    "prax-actix"
    "prax-cli"
)

# Tier 4: Main crate (depends on all)
TIER_4=(
    "prax"
)

# All crates in order
ALL_CRATES=("${TIER_1[@]}" "${TIER_2[@]}" "${TIER_3[@]}" "${TIER_4[@]}")

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

show_help() {
    cat << EOF
Prax Workspace Publish Script

Publishes all crates to crates.io in dependency order to avoid version conflicts.
Automatically skips crates that are already published with the same version.

Usage:
    ./scripts/publish.sh [OPTIONS]

Options:
    --dry-run             Perform a dry run without actually publishing
    --no-verify           Skip cargo verify step (not recommended)
    --allow-dirty         Allow publishing with uncommitted changes
    --skip-version-check  Skip checking if version is already on crates.io
    --version VER         Set version for all crates before publishing
    --delay SEC           Delay between publishes (default: 30 seconds)
    --help                Show this help message

Publish Order:
    Tier 1 (no deps):     ${TIER_1[*]}
    Tier 2 (tier 1 deps): ${TIER_2[*]}
    Tier 3 (tier 2 deps): ${TIER_3[*]}
    Tier 4 (main crate):  ${TIER_4[*]}

Examples:
    # Dry run to see what would happen
    ./scripts/publish.sh --dry-run

    # Publish all crates
    ./scripts/publish.sh

    # Publish with a new version
    ./scripts/publish.sh --version 0.2.0

    # Force publish even if version exists (will fail on crates.io)
    ./scripts/publish.sh --skip-version-check
EOF
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --dry-run)
                DRY_RUN=true
                shift
                ;;
            --no-verify)
                NO_VERIFY=true
                shift
                ;;
            --allow-dirty)
                ALLOW_DIRTY=true
                shift
                ;;
            --skip-version-check)
                SKIP_VERSION_CHECK=true
                shift
                ;;
            --version)
                NEW_VERSION="$2"
                shift 2
                ;;
            --delay)
                DELAY_SECONDS="$2"
                shift 2
                ;;
            --help|-h)
                show_help
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                show_help
                exit 1
                ;;
        esac
    done
}

check_prerequisites() {
    log_info "Checking prerequisites..."

    # Check if cargo is available
    if ! command -v cargo &> /dev/null; then
        log_error "cargo is not installed"
        exit 1
    fi

    # Check if curl is available (needed for version checks)
    if [[ "$SKIP_VERSION_CHECK" == "false" ]]; then
        if ! command -v curl &> /dev/null; then
            log_error "curl is not installed (needed for crates.io version checks)"
            log_info "Install curl or use --skip-version-check to bypass"
            exit 1
        fi
    fi

    # Check if logged in to crates.io
    if ! cargo login --help &> /dev/null; then
        log_error "cargo login not configured. Run 'cargo login' first."
        exit 1
    fi

    # Check for uncommitted changes
    if [[ "$ALLOW_DIRTY" == "false" ]]; then
        if ! git diff --quiet HEAD 2>/dev/null; then
            log_error "Working directory has uncommitted changes. Use --allow-dirty to override."
            exit 1
        fi
    fi

    # Run tests
    log_info "Running tests..."
    if ! cargo test --workspace --lib; then
        log_error "Tests failed. Fix tests before publishing."
        exit 1
    fi

    log_success "Prerequisites check passed"
}

update_version() {
    if [[ -z "$NEW_VERSION" ]]; then
        return
    fi

    log_info "Updating version to $NEW_VERSION..."

    # Update workspace version in root Cargo.toml
    sed -i "s/^version = \".*\"/version = \"$NEW_VERSION\"/" Cargo.toml

    # Update version in workspace.dependencies for internal crates
    for crate in "${ALL_CRATES[@]}"; do
        if [[ "$crate" != "prax" ]]; then
            sed -i "s/${crate} = { path = \"${crate}\", version = \".*\" }/${crate} = { path = \"${crate}\", version = \"$NEW_VERSION\" }/" Cargo.toml
        fi
    done

    # Update each crate's Cargo.toml if they have their own version
    for crate in "${ALL_CRATES[@]}"; do
        local crate_toml
        if [[ "$crate" == "prax" ]]; then
            crate_toml="Cargo.toml"
        else
            crate_toml="$crate/Cargo.toml"
        fi

        if [[ -f "$crate_toml" ]]; then
            # Update version if it's not using workspace
            if grep -q '^version = "' "$crate_toml"; then
                sed -i "s/^version = \".*\"/version = \"$NEW_VERSION\"/" "$crate_toml"
            fi
        fi
    done

    log_success "Version updated to $NEW_VERSION"
}

publish_crate() {
    local crate=$1
    local crate_dir
    local version

    if [[ "$crate" == "prax" ]]; then
        crate_dir="."
    else
        crate_dir="$crate"
    fi

    version=$(get_crate_version "$crate")

    # Check if version is already published (skip in dry-run mode to show all crates)
    if [[ "$DRY_RUN" == "false" && "$SKIP_VERSION_CHECK" == "false" ]]; then
        if ! should_publish_crate "$crate"; then
            log_warn "Skipping $crate v$version - already published on crates.io"
            return 0
        fi
    fi

    log_info "Publishing $crate v$version..."

    local cargo_args=("publish")

    if [[ "$DRY_RUN" == "true" ]]; then
        cargo_args+=("--dry-run")
    fi

    if [[ "$NO_VERIFY" == "true" ]]; then
        cargo_args+=("--no-verify")
    fi

    if [[ "$ALLOW_DIRTY" == "true" ]]; then
        cargo_args+=("--allow-dirty")
    fi

    # Run publish
    if (cd "$crate_dir" && cargo "${cargo_args[@]}"); then
        log_success "Published $crate v$version"
        return 0
    else
        log_error "Failed to publish $crate v$version"
        return 1
    fi
}

wait_for_index() {
    if [[ "$DRY_RUN" == "true" ]]; then
        return
    fi

    log_info "Waiting ${DELAY_SECONDS}s for crates.io to index..."
    sleep "$DELAY_SECONDS"
}

# Get version from a crate's Cargo.toml
get_crate_version() {
    local crate=$1
    local crate_toml

    if [[ "$crate" == "prax" ]]; then
        crate_toml="Cargo.toml"
    else
        crate_toml="$crate/Cargo.toml"
    fi

    # Try to get version from crate's Cargo.toml
    # First check for version.workspace = true, then fall back to root workspace version
    if grep -q 'version.workspace = true' "$crate_toml" 2>/dev/null || grep -q 'version = { workspace = true }' "$crate_toml" 2>/dev/null; then
        # Get version from workspace root
        grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'
    else
        # Get version directly from crate
        grep '^version = ' "$crate_toml" | head -1 | sed 's/version = "\(.*\)"/\1/'
    fi
}

# Check if a specific version of a crate is already published on crates.io
is_version_published() {
    local crate=$1
    local version=$2

    # Query crates.io API for the crate
    local response
    response=$(curl -s "https://crates.io/api/v1/crates/$crate" 2>/dev/null)

    # Check if the response contains the version
    if echo "$response" | grep -q "\"num\":\"$version\""; then
        return 0  # Version is published
    else
        return 1  # Version is not published
    fi
}

# Check if crate needs to be published
should_publish_crate() {
    local crate=$1
    local version

    version=$(get_crate_version "$crate")

    if [[ -z "$version" ]]; then
        log_warn "Could not determine version for $crate, will attempt to publish"
        return 0  # Attempt to publish
    fi

    if is_version_published "$crate" "$version"; then
        return 1  # Already published
    else
        return 0  # Needs publishing
    fi
}

publish_tier() {
    local tier_name=$1
    shift
    local crates=("$@")

    echo ""
    log_info "========================================="
    log_info "Publishing $tier_name"
    log_info "========================================="

    for crate in "${crates[@]}"; do
        if ! publish_crate "$crate"; then
            log_error "Failed to publish $tier_name"
            exit 1
        fi
    done

    # Wait for crates.io to index before publishing next tier
    if [[ ${#crates[@]} -gt 0 ]]; then
        wait_for_index
    fi
}

main() {
    parse_args "$@"

    echo ""
    echo "================================================"
    echo "  Prax Workspace Publish Script"
    echo "================================================"
    echo ""

    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "DRY RUN MODE - No actual publishing will occur"
    fi

    # Change to project root
    cd "$(dirname "$0")/.."

    check_prerequisites
    update_version

    # Publish each tier in order
    publish_tier "Tier 1 (Foundation)" "${TIER_1[@]}"
    publish_tier "Tier 2 (Core Components)" "${TIER_2[@]}"
    publish_tier "Tier 3 (Framework Integrations)" "${TIER_3[@]}"
    publish_tier "Tier 4 (Main Crate)" "${TIER_4[@]}"

    echo ""
    log_success "================================================"
    log_success "  All crates published successfully!"
    log_success "================================================"
    echo ""

    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "This was a dry run. Run without --dry-run to actually publish."
    fi
}

main "$@"

