#!/usr/bin/env bash
# Package an extensions/* folder as VSIX for openvscode-server --install-extension. Author: kejiqing
set -euo pipefail

usage() {
  echo "usage: $0 <extension-src-dir> <out.vsix>" >&2
  exit 1
}

[[ $# -eq 2 ]] || usage
SRC_DIR="$(cd "$1" && pwd)"
OUT_VSIX="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
VERSION="$(python3 -c "import json; p=json.load(open('${SRC_DIR}/package.json')); print(p.get('version','0.1.0'))")"
EXT_ID="$(python3 -c "import json; p=json.load(open('${SRC_DIR}/package.json')); print(p['name'])")"
PUBLISHER="$(python3 -c "import json; p=json.load(open('${SRC_DIR}/package.json')); print(p['publisher'])")"
DISPLAY="$(python3 -c "import json; p=json.load(open('${SRC_DIR}/package.json')); print(p.get('displayName', p['name']))")"
DESC="$(python3 -c "import json; p=json.load(open('${SRC_DIR}/package.json')); print(p.get('description',''))")"

if [[ ! -f "${SRC_DIR}/package.json" ]]; then
  echo "missing ${SRC_DIR}/package.json" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

mkdir -p "${work}/extension"
cp -a "${SRC_DIR}/." "${work}/extension/"

cat >"${work}/extension.vsixmanifest" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">
  <Metadata>
    <Identity Language="en-US" Id="${EXT_ID}" Version="${VERSION}" Publisher="${PUBLISHER}" />
    <DisplayName>${DISPLAY}</DisplayName>
    <Description>${DESC}</Description>
  </Metadata>
  <Installation>
    <InstallationTarget Id="Microsoft.VisualStudio.Code" />
  </Installation>
  <Dependencies />
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />
  </Assets>
</PackageManifest>
EOF

cat >"${work}/[Content_Types].xml" <<'EOF'
<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="json" ContentType="application/json" />
  <Default Extension="vsixmanifest" ContentType="text/xml" />
  <Default Extension="js" ContentType="application/javascript" />
  <Default Extension="xml" ContentType="text/xml" />
</Types>
EOF

rm -f "${OUT_VSIX}"
(
  cd "${work}"
  zip -qr "${OUT_VSIX}" extension extension.vsixmanifest '[Content_Types].xml'
)

echo "wrote ${OUT_VSIX}"
