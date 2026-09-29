"""Find successful main-branch build outputs for the exact release commit."""
import argparse
import json
import os
import re
import subprocess
import time

ARTIFACTS = {
    "haruki-sekai-api-linux-x64",
    "haruki-sekai-api-macos-arm64",
    "haruki-sekai-api-windows-x64",
}


def api(path):
    result = subprocess.run(
        ["gh", "api", path], check=True, capture_output=True, text=True, timeout=60
    )
    return json.loads(result.stdout)


def find_run(repository, sha, kind, request=api, now=time.monotonic, wait=time.sleep):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*", repository):
        raise ValueError("Invalid repository")
    if not re.fullmatch(r"[0-9a-f]{40}", sha) or kind not in {"docker", "release"}:
        raise ValueError("Invalid commit or workflow")
    base = f"repos/{repository}/actions"
    query = f"{base}/workflows/{kind}.yml/runs?branch=main&event=push&head_sha={sha}&per_page=10"
    deadline = now() + 1800
    while True:
        runs = request(query)["workflow_runs"]
        matching = [r for r in runs if r["head_sha"] == sha and r["head_branch"] == "main" and r["event"] == "push"]
        if not matching:
            return None
        run = max(matching, key=lambda r: int(r["id"]))
        if run["status"] == "completed":
            return completed_run_id(run, kind, request, base)
        if now() >= deadline:
            raise TimeoutError("Main build is still running after 30 minutes")
        wait(20)


def completed_run_id(run, kind, request, base):
    if run["conclusion"] == "cancelled":
        return None
    if run["conclusion"] != "success":
        raise RuntimeError(f"Main {kind} build failed; fix or rerun it before releasing")
    run_id = int(run["id"])
    if kind == "release":
        items = request(f"{base}/runs/{run_id}/artifacts?per_page=100")["artifacts"]
        names = {a["name"] for a in items if not a["expired"]}
        if not ARTIFACTS <= names:
            return None
    return run_id


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["docker", "release"])
    args = parser.parse_args()
    run_id = find_run(os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_SHA"], args.kind)
    print(f"reuse={'true' if run_id is not None else 'false'}")
    print(f"run_id={run_id or ''}")


if __name__ == "__main__":
    main()
