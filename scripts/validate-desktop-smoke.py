"""Windows desktop startup/restart smoke test with isolated synthetic 100-node data."""
import argparse
import ctypes
import json
from ctypes import wintypes
import os
import pathlib
import re
import sqlite3
import subprocess
import tempfile
import time
import urllib.request
import uuid


def stop(process):
    if process.poll() is not None:
        return
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]

    @callback_type
    def close_window(handle, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(handle, ctypes.byref(owner))
        if owner.value == process.pid:
            user32.PostMessageW(handle, 0x0010, 0, 0)
        return True

    user32.EnumWindows(close_window, 0)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.terminate()
        process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/release/swiftsub.exe")
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent.parent
    binary = pathlib.Path(args.binary).resolve(strict=True)
    schema_source = (root / "crates/swiftsub-store/src/lib.rs").read_text(encoding="utf-8")
    schema = re.search(r'const SCHEMA: &str = r#"(.*?)"#;', schema_source, re.S).group(1)
    validation = root / "target/validation"
    validation.mkdir(parents=True, exist_ok=True)
    fixture = pathlib.Path(tempfile.mkdtemp(prefix="desktop-", dir=validation))
    app_directory = fixture / "SwiftSub"
    app_directory.mkdir()
    database = app_directory / "swiftsub.db"
    config_id = uuid.uuid4().hex
    saved_yaml = "proxies:\n" + "".join(
        f"  - {{name: Node-{i}, type: trojan, server: node-{i}.example.org, port: 443, password: smoke-only}}\n"
        for i in range(100)
    ) + "proxy-groups: [{name: Select, type: select, proxies: [Node-0, DIRECT]}]\nrules: ['MATCH,Select']\n"
    with sqlite3.connect(database) as connection:
        connection.executescript(schema)
        connection.execute("PRAGMA user_version=3")
        links = "\n".join(f"trojan://smoke-only@node-{i}.example.org:443#Node-{i}" for i in range(100))
        now = int(time.time())
        connection.execute(
            "INSERT INTO subscriptions(name,kind,payload,node_count,created_at,updated_at) VALUES(?,?,?,?,?,?)",
            ("100 synthetic nodes", "links", links, 100, now, now),
        )
        connection.execute(
            "INSERT INTO saved_configs(id,name,yaml,created_at,updated_at) VALUES(?,?,?,?,?)",
            (config_id, "100-node saved config", saved_yaml, now, now),
        )
    environment = dict(os.environ, SWIFTSUB_DATA_DIR=str(app_directory), APPDATA=str(fixture))
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    previous_body = None
    previous_pull = None
    for phase in ("startup", "restart"):
        with (fixture / f"{phase}.log").open("wb") as log:
            process = subprocess.Popen(
                [str(binary), "--background", "--page=generator"], env=environment,
                startupinfo=startup, stdout=log, stderr=subprocess.STDOUT,
            )
            try:
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise RuntimeError(f"desktop exited: {process.returncode}; see {fixture}")
                    try:
                        with sqlite3.connect(database, timeout=1) as connection:
                            token, yaml = connection.execute("SELECT token,last_yaml FROM profiles ORDER BY id LIMIT 1").fetchone()
                            port = connection.execute("SELECT value FROM settings WHERE key='server_port'").fetchone()[0]
                        with opener.open(f"http://127.0.0.1:{port}/health", timeout=1) as response:
                            assert response.read() == b"ok"
                        with opener.open(f"http://127.0.0.1:{port}/sub/{token}/test.yaml", timeout=1) as response:
                            body = response.read().decode("utf-8")
                        with opener.open(f"http://127.0.0.1:{port}/sub/{config_id}/config.yaml", timeout=1) as response:
                            assert response.read().decode("utf-8") == saved_yaml
                        with sqlite3.connect(database, timeout=1) as connection:
                            assert connection.execute("PRAGMA user_version").fetchone()[0] == 4
                            assert connection.execute("SELECT yaml FROM config_revisions WHERE config_id=?", (config_id,)).fetchone()[0] == saved_yaml
                            pull_row = connection.execute("SELECT value FROM settings WHERE key=?", (f"client-pull:{config_id}",)).fetchone()
                            assert pull_row is not None
                            pull = json.loads(pull_row[0])
                            assert pull[0] > 0 and pull[1]
                            if previous_pull is not None:
                                assert pull[0] >= previous_pull[0] and pull[1] == previous_pull[1]
                        assert body == yaml
                        assert len(re.findall(r"(?m)^\s+server: node-\d+\.example\.org$", body)) == 100
                        if previous_body is not None:
                            assert body == previous_body
                        previous_body = body
                        previous_pull = pull
                        print(f"{phase}: PASS (100 nodes, HTTP output and persisted pull history verified)")
                        break
                    except (OSError, sqlite3.Error, TypeError, AssertionError):
                        time.sleep(0.1)
                else:
                    raise RuntimeError(f"desktop smoke timed out; see {fixture}")
            finally:
                stop(process)
    print(f"Isolated fixture: {fixture}")


if __name__ == "__main__":
    main()
