"""Prueba de aceptación del wrapper MCP, con stdout/stderr separados.

No necesita paquetes de Python. Ejecutar después de scripts/build.
"""
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
INIT = {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"s0-acceptance","version":"0"}}}

def command(workspace):
    if os.name == "nt":
        return [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/c", r"scripts\mcp.bat"]
    return ["bash", str(ROOT / "scripts/mcp.sh"), "--workspace", str(workspace)]

def main():
    with tempfile.TemporaryDirectory(prefix="runpost-s0-") as workspace:
        single = subprocess.run(command(workspace), cwd=ROOT, env=dict(os.environ, RUNPOST_WORKSPACE=workspace), input=json.dumps(INIT) + "\n", text=True, capture_output=True, timeout=20, check=True)
        lines = single.stdout.splitlines()
        assert len(lines) == 1, f"Expected one JSON-RPC line, got {single.stdout!r}"
        assert json.loads(lines[0])["result"]["protocolVersion"] == "2025-06-18"
        assert "MCP stdio started" in single.stderr
        print("Single initialize + EOF: valid JSON-RPC, exactly 1 stdout line, exit 0")
        log = Path(workspace, "logs/mcp.log")
        assert "MCP stdio started" in log.read_text(encoding="utf-8")

        with subprocess.Popen(command(workspace), cwd=ROOT, env=dict(os.environ, RUNPOST_WORKSPACE=workspace), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1) as process:
            replies = queue.Queue()
            reader = threading.Thread(target=lambda: [replies.put(line) for line in process.stdout], daemon=True)
            reader.start()
            try:
                start = time.perf_counter()
                process.stdin.write(json.dumps(INIT) + "\n")
                process.stdin.flush()
                init = json.loads(replies.get(timeout=15))
                elapsed = (time.perf_counter() - start) * 1000
                assert init["id"] == 1 and init["result"]["serverInfo"]["name"] == "runpost"
                for value in [{"jsonrpc":"2.0","method":"notifications/initialized"}, {"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}]:
                    process.stdin.write(json.dumps(value) + "\n")
                process.stdin.flush()
                tools = json.loads(replies.get(timeout=15))
                assert tools["id"] == 2 and len(tools["result"]["tools"]) == 13
                process.stdin.close()
                assert process.wait(timeout=15) == 0
                reader.join(timeout=5)
                assert replies.empty(), "Unexpected extra stdout"
                print(f"Interactive initialize + tools/list: 13 tools, no extra stdout ({elapsed:.1f} ms for wrapper initialization)")
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
                process.stderr.close()

if __name__ == "__main__":
    main()
