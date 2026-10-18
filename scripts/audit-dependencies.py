"""Audita lockfiles npm/Cargo con OSV, cooldown npm y paquetes huérfanos.

Python 3.11+, solo biblioteca estándar. No instala ni ejecuta dependencias.
"""
import concurrent.futures
import datetime as dt
import json
from pathlib import Path
import sys
import tomllib
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
NOW = dt.datetime.now(dt.timezone.utc)
HEADERS = {"User-Agent": "RunPost-dependency-audit/0.1"}

def get_json(url, data=None):
    headers = dict(HEADERS)
    if data is not None:
        data = json.dumps(data).encode()
        headers["Content-Type"] = "application/json"
    with urllib.request.urlopen(urllib.request.Request(url, data=data, headers=headers), timeout=45) as response:
        return json.load(response)

def main():
    queries = {}
    npm_versions = set()
    errors = []
    for folder in (ROOT / "ui", ROOT / "apps/desktop"):
        lock = json.loads((folder / "package-lock.json").read_text(encoding="utf-8"))
        locked = set()
        for key, metadata in lock["packages"].items():
            if not key:
                continue
            name = key.rsplit("node_modules/", 1)[-1]
            version = metadata["version"]
            locked.add((name, version))
            npm_versions.add((name, version))
            queries[("npm", name, version)] = {"package": {"name": name, "ecosystem": "npm"}, "version": version}
            if not metadata.get("integrity") or not metadata.get("resolved", "").startswith("https://registry.npmjs.org/"):
                errors.append(f"Missing integrity or unexpected registry: {name}@{version}")
        installed = set()
        for manifest in (folder / "node_modules").rglob("package.json"):
            # Solo raíces de paquetes, excluyendo fixtures y metadatos internos.
            if manifest.parent.parent.name != "node_modules" and not (
                manifest.parent.parent.name.startswith("@") and manifest.parent.parent.parent.name == "node_modules"
            ):
                continue
            meta = json.loads(manifest.read_text(encoding="utf-8"))
            installed.add((meta["name"], meta["version"]))
        orphans = installed - locked
        if orphans:
            errors.append(f"Orphans in {folder.relative_to(ROOT)}: {sorted(orphans)}")
        print(f"{folder.relative_to(ROOT)}: {len(locked)} locked, {len(installed)} installed, {len(orphans)} orphans", flush=True)

    cargo = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    for package in cargo["package"]:
        if not package.get("source", "").startswith("registry+"):
            continue
        name, version = package["name"], package["version"]
        if not package.get("checksum"):
            errors.append(f"Missing Cargo checksum: {name}@{version}")
        queries[("crates.io", name, version)] = {"package": {"name": name, "ecosystem": "crates.io"}, "version": version}

    keys = list(queries)
    print(f"OSV: checking {len(keys)} exact npm/Cargo packages", flush=True)
    findings = []
    for start in range(0, len(keys), 400):
        batch = keys[start:start+400]
        response = get_json("https://api.osv.dev/v1/querybatch", {"queries": [queries[key] for key in batch]})
        for key, result in zip(batch, response["results"], strict=True):
            if result.get("vulns"):
                findings.append({"ecosystem": key[0], "name": key[1], "version": key[2], "ids": [v["id"] for v in result["vulns"]]})

    def age(item):
        name, version = item
        metadata = get_json("https://registry.npmjs.org/" + urllib.parse.quote(name, safe=""))
        published = dt.datetime.fromisoformat(metadata["time"][version].replace("Z", "+00:00"))
        days = (NOW - published).total_seconds() / 86400
        return {"name": name, "version": version, "published": published.isoformat(), "age_days": round(days, 2)}

    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as executor:
        ages = list(executor.map(age, sorted(npm_versions)))
    for entry in ages:
        if entry["age_days"] < 7:
            errors.append(f"Cooldown: {entry['name']}@{entry['version']} has {entry['age_days']} days")
    report = {"checked_at_utc": NOW.isoformat(), "osv_packages": len(keys), "findings": findings, "npm_ages": ages, "errors": errors}
    out = ROOT / "artifacts/dependency-audit.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"OSV findings: {len(findings)}; cooldown/integrity/orphan errors: {len(errors)}")
    for finding in findings:
        print(json.dumps(finding))
    for error in errors:
        print(error)
    print(f"Report: {out}")
    return 1 if findings or errors else 0

if __name__ == "__main__":
    sys.exit(main())
