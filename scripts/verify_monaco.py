"""Exercise locally built Monaco, real Ruff LSP, revision saves and binary packets."""

import argparse
import hashlib
import json
import os
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("assets", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    assets = args.assets.resolve()

    def digest(p):
        return hashlib.sha256(Path(p).read_bytes()).hexdigest()

    ruff = Path(shutil.which("ruff") or "missing").resolve()
    assert ruff.is_file(), "real Ruff server required"
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / "sample.py"
        source.write_text("import os\nprint(undefined_name)\n")
        config = root / "servers.json"
        config.write_text(
            json.dumps(
                [
                    {
                        "schema_version": 1,
                        "id": "ruff",
                        "program": str(ruff),
                        "program_sha256": digest(ruff),
                        "args": ["server"],
                        "language_id": "python",
                        "timeout_seconds": 20,
                    }
                ]
            )
        )
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        token = "monaco-local-verification-token-0123456789"
        env = {
            **os.environ,
            "AEGIS_OPERATOR_TOKEN": token,
            "AEGIS_EDITOR_ASSETS": str(assets),
            "AEGIS_EDITOR_ASSETS_MANIFEST_SHA256": digest(assets / "manifest.json"),
            "AEGIS_LANGUAGE_SERVERS": str(config),
        }
        server = subprocess.Popen(
            [
                binary,
                "operator-serve",
                root / "ledger.db",
                root,
                "--bind",
                f"127.0.0.1:{port}",
                "--trusted-local",
            ],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        try:
            for _ in range(200):
                try:
                    with urllib.request.urlopen(base + "/health", timeout=1):
                        break
                except urllib.error.URLError:
                    if server.poll() is not None:
                        raise RuntimeError(server.stderr.read().decode())
                    time.sleep(0.05)
            else:
                raise AssertionError("server did not start")
            for path in [
                "/editor-assets/manifest.json",
                "/editor-assets/../servers.json",
                "/editor-assets/missing.js",
            ]:
                try:
                    urllib.request.urlopen(base + path)
                except urllib.error.HTTPError as error:
                    assert error.code == 404
                else:
                    raise AssertionError("unlisted asset exposed")
            payload = root / "payload.bin"
            payload.write_bytes(b"\x00raw\xffcontent-ID\x00")
            frame = root / "packet.aeg"
            subprocess.run([binary, "wire-pack", payload, frame], check=True, capture_output=True)
            subprocess.run(
                [binary, "wire-unpack", frame, root / "restored.bin"],
                check=True,
                capture_output=True,
            )
            assert (root / "restored.bin").read_bytes() == payload.read_bytes()
            request = urllib.request.Request(
                base + "/v1/wire/validate",
                data=frame.read_bytes(),
                headers={
                    "Authorization": "Bearer " + token,
                    "Content-Type": "application/octet-stream",
                },
            )
            with urllib.request.urlopen(request) as response:
                packet = json.load(response)
            assert packet["content_id_wire_bytes"] == 32
            errors, external = [], []
            with sync_playwright() as playwright:
                browser = playwright.chromium.launch(headless=True)
                page = browser.new_page(viewport={"width": 1280, "height": 950})
                page.on("pageerror", lambda error: errors.append(str(error)))
                page.on(
                    "request",
                    lambda req: (
                        external.append(req.url)
                        if not req.url.startswith((base, "data:", "blob:"))
                        else None
                    ),
                )
                page.goto(base + "/workspace")
                page.locator("#token").fill(token)
                page.locator("#load").click()
                page.locator("#files button", has_text="sample.py").click()
                page.locator("#toggle-monaco").click()
                page.locator(".monaco-editor").wait_for(timeout=30000)
                page.locator("#language-server").fill("ruff")
                page.locator("#diagnose").click()
                page.wait_for_function(
                    "document.getElementById('diagnostics').textContent.includes('F821')",
                    timeout=30000,
                )
                page.locator(".squiggly-error").first.wait_for()
                page.screenshot(path=str(args.out.with_suffix(".png")), full_page=True)
                page.locator(".monaco-editor").click()
                page.keyboard.press("Control+a")
                page.keyboard.insert_text("print(42)\n")
                page.wait_for_function(
                    "document.getElementById('identity').textContent.includes('unsaved')"
                )
                page.locator("#diagnose").click()
                assert "Save this revision" in page.locator("#message").inner_text()
                page.locator("#save").click()
                page.wait_for_function(
                    "document.getElementById('message').textContent.startsWith('Saved')"
                )
                assert source.read_text() == "print(42)\n"
                page.locator("#diagnose").click()
                page.wait_for_function(
                    "document.getElementById('diagnostics').textContent === 'No diagnostics for this saved revision'",
                    timeout=30000,
                )
                page.locator("#toggle-monaco").click()
                assert page.locator("#source").input_value() == "print(42)\n"
                assert not external, external
                assert not errors, errors
                browser.close()
        finally:
            server.terminate()
            server.wait(timeout=10)
    receipt = {
        "schema_version": 1,
        "local_monaco_loaded_without_external_requests": True,
        "real_ruff_diagnostics_rendered_as_markers": True,
        "monaco_edit_save_and_plain_editor_toggle": True,
        "unsaved_diagnostics_rejected": True,
        "unlisted_asset_paths_rejected": True,
        "raw_32_byte_ids_cli_api_round_trip": True,
        "scope": "local Monaco + bounded saved-file LSP bridge; not persistent monaco-languageclient sessions, DAP or a full IDE",
    }
    args.out.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
