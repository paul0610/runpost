# Mide los objetivos de arranque de la sección 14 con los binarios release:
#   - app Tauri: del arranque del proceso a la ventana usable (la SPA cargada pide `backend_info`;
#     la app lo anota con RUNPOST_STARTUP_LOG). Objetivo < 1,5 s.
#   - `runpost_mcp mcp`: del arranque a la respuesta de `initialize`. Objetivo < 100 ms.
#   - memoria en reposo del proceso de la app (sin los procesos del webview), 3 s después de la
#     ventana usable. Objetivo < 60 MB.
# Usa un workspace temporal y el llavero en memoria. Abre y cierra la ventana de la app varias veces,
# así que no corre si RunPost ya está abierto (instancia única).
# Uso: python scripts/dev/startup_time.py [--runs 3]
import argparse
import json
import os
import pathlib
import re
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
EXE = '.exe' if os.name == 'nt' else ''
APP = ROOT / 'target' / 'release' / f'runpost_app{EXE}'
CLI = ROOT / 'target' / 'release' / f'runpost_mcp{EXE}'


def app_running():
    if os.name == 'nt':
        out = subprocess.run(['tasklist', '/FI', 'IMAGENAME eq runpost_app.exe'], capture_output=True, text=True).stdout
        return 'runpost_app.exe' in out
    return subprocess.run(['pgrep', '-x', 'runpost_app'], capture_output=True).returncode == 0


def resident_mb(pid):
    """Memoria residente (working set en Windows, RSS en macOS/Linux) de un proceso, en MB."""
    if os.name == 'nt':
        out = subprocess.run(['tasklist', '/FI', f'PID eq {pid}', '/FO', 'CSV', '/NH'],
                             capture_output=True, text=True).stdout
        match = re.search(r'"([\d.,\s]+) K', out)
        return int(re.sub(r'\D', '', match.group(1))) / 1024 if match else None
    out = subprocess.run(['ps', '-o', 'rss=', '-p', str(pid)], capture_output=True, text=True).stdout
    return int(out.strip()) / 1024 if out.strip() else None


def measure_app(workspace, timeout=20):
    env = {**os.environ, 'RUNPOST_STARTUP_LOG': '1', 'RUNPOST_KEYRING': 'memory'}
    started = time.perf_counter()
    process = subprocess.Popen([str(APP), '--workspace', str(workspace)], env=env,
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    result = {}

    def read():
        for line in process.stderr:
            match = re.search(r'startup: ui ready after (\d+) ms', line)
            if match:
                result['app_ms'] = int(match.group(1))
                result['wall_ms'] = round((time.perf_counter() - started) * 1000)
                return

    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    reader.join(timeout)
    if 'app_ms' in result:
        time.sleep(3)
        result['mem_mb'] = resident_mb(process.pid)
    process.kill()
    process.wait()
    return result


def measure_mcp(workspace):
    env = {**os.environ, 'RUNPOST_KEYRING': 'memory'}
    started = time.perf_counter()
    process = subprocess.Popen([str(CLI), 'mcp', '--workspace', str(workspace)], env=env,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    process.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
        'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'bench', 'version': '0'}}}) + '\n')
    process.stdin.flush()
    process.stdout.readline()
    elapsed = round((time.perf_counter() - started) * 1000)
    process.stdin.close()
    process.wait(timeout=10)
    return elapsed


def summary(values):
    return f'{min(values)} / {round(statistics.median(values))} / {max(values)} ms' if values else 'sin datos'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--runs', type=int, default=3)
    args = parser.parse_args()
    for binary in (APP, CLI):
        if not binary.exists():
            sys.exit(f'falta {binary}: cargo build --release')
    workspace = pathlib.Path(tempfile.mkdtemp(prefix='runpost-startup-'))
    (workspace / 'settings.json').write_text(json.dumps({'mcp': {'http_port': None}}), encoding='utf-8')
    mcp = [measure_mcp(workspace) for _ in range(args.runs)]
    app, wall, memory = [], [], []
    if app_running():
        print('RunPost está abierto: no mido la app (instancia única). Ciérralo y repite.')
    else:
        for _ in range(args.runs):
            result = measure_app(workspace)
            if 'app_ms' in result:
                app.append(result['app_ms'])
                wall.append(result['wall_ms'])
                if result.get('mem_mb'):
                    memory.append(result['mem_mb'])
            time.sleep(1)
    print('| Medida (mín / mediana / máx) | Objetivo | Resultado |')
    print('|---|---|---|')
    print(f'| App: arranque hasta la ventana usable (proceso) | < 1 500 ms | {summary(app)} |')
    print(f'| App: lo mismo desde que se lanza el ejecutable | < 1 500 ms | {summary(wall)} |')
    print(f'| `runpost_mcp mcp` hasta responder `initialize` | < 100 ms | {summary(mcp)} |')
    mem = f'{min(memory):.0f} / {statistics.median(memory):.0f} / {max(memory):.0f} MB' if memory else 'sin datos'
    print(f'| App en reposo: memoria del proceso (sin el webview) | < 60 MB | {mem} |')
    for name in (APP, CLI):
        print(f'| Tamaño de `{name.name}` | {"< 25 MB" if name == APP else "< 15 MB"} | {name.stat().st_size / 1_000_000:.1f} MB |')


if __name__ == '__main__':
    main()
