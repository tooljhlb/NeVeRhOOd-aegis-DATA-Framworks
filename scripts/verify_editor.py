"""Exercise native editor APIs and a real browser coding workflow."""

import argparse
import concurrent.futures
import json
import os
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

p = argparse.ArgumentParser()
p.add_argument("binary", type=Path)
p.add_argument("--out", type=Path, required=True)
a = p.parse_args()
binary = a.binary.resolve()
with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    (root / "sample.py").write_text("value = 1\n")
    (root / "other.py").write_text("other = 2\n")
    (root / ".env").write_text("DO_NOT_SHOW=secret")
    (root / "link.py").symlink_to(root / "sample.py")
    for args in [
        ["init", "-q"],
        ["add", "sample.py", "other.py"],
        [
            "-c",
            "user.name=AEGIS Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "baseline",
        ],
    ]:
        subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    token = "editor-test-token-abcdefghijklmnopqrstuvwxyz"
    env = dict(os.environ, AEGIS_OPERATOR_TOKEN=token)
    process = subprocess.Popen(
        [
            str(binary),
            "operator-serve",
            str(root / "operator.db"),
            str(root),
            "--bind",
            f"127.0.0.1:{port}",
            "--trusted-local",
        ],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )

    def request(path, data=None, bearer=token):
        req = urllib.request.Request(
            base + path,
            data=None if data is None else json.dumps(data).encode(),
            headers={"Authorization": f"Bearer {bearer}", "Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=10) as response:
            return json.load(response)

    try:
        for _ in range(100):
            try:
                request("/health")
                break
            except OSError:
                time.sleep(0.05)
        original = request("/v1/workspace/file?path=sample.py")

        def save(n):
            try:
                request(
                    "/v1/workspace/file",
                    {
                        "path": "sample.py",
                        "expected_sha256": original["sha256"],
                        "text": f"value = {n}\n",
                    },
                )
                return 200
            except urllib.error.HTTPError as error:
                return error.code

        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
            codes = list(pool.map(save, range(10, 16)))
        assert codes.count(200) == 1 and codes.count(409) == 5, codes
        current = request("/v1/workspace/file?path=sample.py")
        request(
            "/v1/workspace/file",
            {
                "path": "sample.py",
                "expected_sha256": current["sha256"],
                "snapshot": original["sha256"],
            },
        )
        for path, bearer, code in [
            ("/v1/workspace/file?path=../outside", token, 400),
            ("/v1/workspace/file?path=.env", token, 400),
            ("/v1/workspace/file?path=link.py", token, 400),
            ("/v1/workspace/tree", "wrong", 401),
        ]:
            try:
                request(path, bearer=bearer)
                raise AssertionError("editor boundary bypass")
            except urllib.error.HTTPError as error:
                assert error.code == code
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(headless=True, args=["--no-sandbox"])
            page = browser.new_page(viewport={"width": 1280, "height": 900})
            errors = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            page.goto(base + "/workspace")
            page.locator("#token").fill(token)
            page.locator("#load").click()
            page.locator("#files button").filter(has_text="sample.py").click()
            page.locator("#source").fill("value = 3\n")
            page.locator("#save").click()
            page.wait_for_function(
                "document.querySelector('#message').textContent==='Saved sample.py'"
            )
            assert (root / "sample.py").read_text() == "value = 3\n"
            page.locator("#files button").filter(has_text="other.py").click()
            page.wait_for_function("document.querySelector('#tabs').children.length===2")
            page.locator("#tabs button").filter(has_text="sample.py").click()
            page.locator("#query").fill("value = 3")
            page.locator("#search").click()
            page.wait_for_function(
                "document.querySelector('#matches').textContent.includes('sample.py:1')"
            )
            page.locator("#git-diff").click()
            page.wait_for_function(
                "document.querySelector('#git-output').textContent.includes('+value = 3')"
            )
            page.locator("#source").fill("value = 4\n")
            (root / "sample.py").write_text("value = 5\n")
            page.locator("#save").click()
            page.wait_for_function(
                "document.querySelector('#message').textContent.includes('edit conflict')"
            )
            assert page.locator("#source").input_value() == "value = 4\n"
            assert (root / "sample.py").read_text() == "value = 5\n"
            page.on("dialog", lambda dialog: dialog.accept())
            page.locator("#reload").click()
            page.wait_for_function("document.querySelector('#source').value==='value = 5\\n'")
            page.locator("#source").fill("value = 6\n")
            page.locator("#save").click()
            page.wait_for_function(
                "document.querySelector('#message').textContent==='Saved sample.py'"
            )
            page.locator("#rollback").click()
            page.wait_for_function(
                "document.querySelector('#message').textContent==='Restored snapshot'"
            )
            assert (root / "sample.py").read_text() == "value = 5\n"
            page.locator("#root").fill(str(root))
            page.locator("#program").fill("python3")
            page.locator("#args").fill(
                json.dumps(
                    [
                        "-c",
                        "import runpy; assert runpy.run_path('sample.py')['value'] == 5; print(12345)",
                    ]
                )
            )
            page.locator("#run-command").click()
            page.wait_for_function(
                "document.querySelector('#command-output').textContent.includes('12345')",
                timeout=15000,
            )
            assert not errors, errors
            page.screenshot(path=str(a.out.with_suffix(".png")), full_page=True)
            browser.close()
    finally:
        process.terminate()
        process.wait(timeout=10)
receipt = {
    "concurrent_stale_writes_rejected": True,
    "snapshot_rollback_verified": True,
    "auth_path_secret_and_symlink_boundaries": True,
    "browser_tree_tabs_edit_save_search_git_diff": True,
    "browser_conflict_retains_unsaved_buffer": True,
    "browser_reload_and_snapshot_restore": True,
    "browser_command_uses_durable_operator": True,
    "scope": "trusted local plain-text editor; not full IDE, uncontrolled external-editor CAS, multi-file transaction or hosted tenancy certification",
}
a.out.write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps(receipt))
