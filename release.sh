#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

usage() {
    echo "Usage: $0 <version>"
    echo ""
    echo "Examples:"
    echo "  $0 0.1.0"
    echo "  $0 1.0.0-beta.1"
    echo ""
    echo "This script will:"
    echo "  1. Validate the version format"
    echo "  2. Update Cargo.toml version"
    echo "  3. Commit the version change"
    echo "  4. Create and push a git tag (v<version>)"
    echo "  5. Trigger the GitHub Actions release workflow"
    exit 1
}

# Check for version argument
if [ -z "$1" ]; then
    echo -e "${RED}Error: Version argument required${NC}"
    usage
fi

VERSION="$1"
TAG="v$VERSION"

# Validate version format (semver-ish)
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
    echo -e "${RED}Error: Invalid version format '$VERSION'${NC}"
    echo "Version must be in format: X.Y.Z or X.Y.Z-prerelease"
    exit 1
fi

# Check if we're in the right directory
if [ ! -f "Cargo.toml" ]; then
    echo -e "${RED}Error: Cargo.toml not found. Run this script from the project root.${NC}"
    exit 1
fi

# Check for uncommitted changes
if ! git diff-index --quiet HEAD --; then
    echo -e "${YELLOW}Warning: You have uncommitted changes.${NC}"
    read -p "Continue anyway? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# Check if tag already exists
if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo -e "${RED}Error: Tag $TAG already exists${NC}"
    exit 1
fi

echo -e "${GREEN}Releasing version $VERSION${NC}"
echo ""

# Update version in Cargo.toml
echo "Updating Cargo.toml version..."
if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
else
    sed -i "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
fi

# Show the change
echo "Updated Cargo.toml:"
grep "^version = " Cargo.toml

# Commit the version change
echo ""
echo "Committing version change..."
git add Cargo.toml
git commit -m "Release $TAG"

# Create the tag
echo ""
echo "Creating tag $TAG..."
git tag -a "$TAG" -m "Release $TAG"

# Push
echo ""
echo "Pushing to remote..."
git push origin main
git push origin "$TAG"

echo ""
echo -e "${GREEN}Release $TAG initiated!${NC}"
echo ""
echo "GitHub Actions will now build and publish the release."
echo "Monitor progress at: https://github.com/danbruder/litehouse/actions"
echo ""
echo "Once complete, the release will be available at:"
echo "https://github.com/danbruder/litehouse/releases/tag/$TAG"
