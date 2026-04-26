# Icons

Tauri expects these icon files at build time. For v0.1 you can generate placeholder icons via:

```bash
# From any source PNG (recommended: 1024x1024 black anchor on transparent background)
brew install imagemagick   # if not installed
cd src-tauri/icons

# macOS .icns
mkdir icon.iconset
sips -z 16 16     source.png --out icon.iconset/icon_16x16.png
sips -z 32 32     source.png --out icon.iconset/icon_16x16@2x.png
sips -z 32 32     source.png --out icon.iconset/icon_32x32.png
sips -z 64 64     source.png --out icon.iconset/icon_32x32@2x.png
sips -z 128 128   source.png --out icon.iconset/icon_128x128.png
sips -z 256 256   source.png --out icon.iconset/icon_128x128@2x.png
sips -z 256 256   source.png --out icon.iconset/icon_256x256.png
sips -z 512 512   source.png --out icon.iconset/icon_256x256@2x.png
sips -z 512 512   source.png --out icon.iconset/icon_512x512.png
sips -z 1024 1024 source.png --out icon.iconset/icon_512x512@2x.png
iconutil -c icns icon.iconset
rm -rf icon.iconset

# Windows .ico (if cross-building)
convert source.png -resize 256x256 icon.ico

# Standard PNG sizes
convert source.png -resize 32x32   32x32.png
convert source.png -resize 128x128 128x128.png
cp source.png icon.png
```

For development, you can use a placeholder until the real anchor logo is ready — Tauri will warn but build.
