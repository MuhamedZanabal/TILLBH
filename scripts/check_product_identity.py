#!/usr/bin/env python3
import argparse, json, pathlib, sys, tomllib

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--root", default=".")
    root=pathlib.Path(ap.parse_args().root)
    errors=[]

    def require(ok, message):
        if not ok:
            errors.append(message)

    package=json.loads((root/"package.json").read_text(encoding="utf-8"))
    require(package.get("name")=="tillbahrain", "package.json name must be tillbahrain")
    require(package.get("version")=="1.0.0", "package.json version must be 1.0.0")

    cargo=tomllib.loads((root/"Cargo.toml").read_text(encoding="utf-8"))
    wp=cargo.get("workspace",{}).get("package",{})
    require(wp.get("version")=="1.0.0", "Cargo workspace version must be 1.0.0")
    require(wp.get("rust-version")=="1.82", "Rust minimum must be 1.82")

    desktop=tomllib.loads((root/"src-tauri"/"Cargo.toml").read_text(encoding="utf-8"))
    require(desktop.get("package",{}).get("name")=="tillbahrain", "desktop crate must be tillbahrain")

    conf=json.loads((root/"src-tauri"/"tauri.conf.json").read_text(encoding="utf-8"))
    for k,v in {"productName":"Tillbahrain","version":"1.0.0","identifier":"com.tillbahrain.pos"}.items():
        require(conf.get(k)==v, f"tauri.conf.json {k} must be {v}")
    require(conf.get("bundle",{}).get("publisher")=="Tillbahrain", "bundle publisher must be Tillbahrain")

    build=(root/"src-tauri"/"build.rs").read_text(encoding="utf-8")
    shell=(root/"src-tauri"/"src"/"lib.rs").read_text(encoding="utf-8")
    smoke=(root/"scripts"/"installer-smoke.ps1").read_text(encoding="utf-8")
    require("TILLBAHRAIN_BUILD_SHA" in build and "TILLBAHRAIN_BUILD_SHA" in shell, "build SHA metadata must use TILLBAHRAIN_BUILD_SHA")
    require('"Tillbahrain starting"' in shell, "startup log must identify Tillbahrain")
    require("Tillbahrain_" in smoke and "_x64-setup.exe" in smoke, "smoke test must require Tillbahrain installer filename")
    require("tillbahrain.exe" in smoke, "smoke test must target tillbahrain.exe")

    if errors:
        for e in errors:
            print(f"IDENTITY CONTRACT: {e}", file=sys.stderr)
        return 1
    print("Tillbahrain product identity contract: PASS")
    return 0

if __name__=="__main__":
    raise SystemExit(main())
