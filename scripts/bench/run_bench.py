# Benchmark real del sprint 7 (ARCHITECTURE 14 y 19.1): la misma tarea de Acme con Claude Code
# usando curl frente a usando RunPost (MCP), con y sin la skill `api-testing` del plugin.
#
# Cada brazo corre `claude -p` en una carpeta temporal nueva, contra el mock de Acme
# (scripts/dev/mock_acme.py, que este script arranca si no está), y guarda lo que reporta el propio
# cliente (`--output-format json`: tokens de entrada, caché, salida, turnos, coste) y, en los brazos
# de RunPost, el medidor del panel Agente (`GET /api/agent`: bytes crudos frente a enviados).
#
# OJO: cada brazo es una sesión real de Claude Code con TU cuenta (gasta tokens). `--dry-run`
# muestra lo que haría sin llamar a Claude.
#
# Uso:
#   python scripts/bench/run_bench.py --dry-run
#   python scripts/bench/run_bench.py --tasks t1,t2 --arms curl,runpost,runpost-skill [--model M] [--runs 1]
import argparse
import datetime
import json
import os
import pathlib
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]
EXE = '.exe' if os.name == 'nt' else ''
CLI = ROOT / 'target' / 'release' / f'runpost_mcp{EXE}'
COLLECTION = ROOT / 'tests' / 'fixtures' / 'Acme-Gateway-Scopes.postman_collection.json'
ENVIRONMENT = ROOT / 'tests' / 'fixtures' / 'Acme-DESA-fake.postman_environment.json'
PLUGIN = ROOT / 'plugin'
TASKS_DIR = pathlib.Path(__file__).resolve().parent / 'tasks'
API_PORT, MCP_PORT = 7891, 7892

ARMS = {
    'curl': 'Claude Code con curl (sin MCP), como la línea base',
    'runpost': 'Claude Code con las tools de RunPost por MCP',
    'runpost-skill': 'Igual, más la skill api-testing del plugin',
}


def port_open(port):
    with socket.socket() as s:
        s.settimeout(0.3)
        return s.connect_ex(('127.0.0.1', port)) == 0


def ensure_mock(dry):
    if port_open(3080) and port_open(3081):
        return None
    print('mock Acme: arrancando scripts/dev/mock_acme.py (3080/3081)')
    if dry:
        return None
    mock = subprocess.Popen([sys.executable, '-I', str(ROOT / 'scripts' / 'dev' / 'mock_acme.py')])
    for _ in range(50):
        if port_open(3080) and port_open(3081):
            return mock
        time.sleep(0.1)
    sys.exit('el mock de Acme no arrancó')


def prompt(task, arm):
    text = (TASKS_DIR / f'{task}.md').read_text(encoding='utf-8')
    tool_hint = (TASKS_DIR / ('curl.md' if arm == 'curl' else 'runpost.md')).read_text(encoding='utf-8')
    return f'{text.strip()}\n\n{tool_hint.strip()}\n'


def prepare_runpost(project):
    """Workspace con la colección y el entorno falso (apuntando al mock) y el servidor de la app."""
    ws = project / 'workspace'
    env = {**os.environ, 'RUNPOST_KEYRING': 'memory'}
    for fixture in (COLLECTION, ENVIRONMENT):
        subprocess.run([str(CLI), '--workspace', str(ws), 'import', str(fixture)], check=True,
                       env=env, capture_output=True)
    settings_path = ws / 'settings.json'
    settings = json.loads(settings_path.read_text(encoding='utf-8'))
    env_file = next((ws / 'environments').glob('*.env.json'))
    env_doc = json.loads(env_file.read_text(encoding='utf-8'))
    for var in env_doc['variables']:
        if var['key'] == 'gatewayUrl':
            var['value'] = 'http://127.0.0.1:3080'
        if var['key'] == 'apiUrl':
            var['value'] = 'http://127.0.0.1:3081'
    env_file.write_text(json.dumps(env_doc, indent=2), encoding='utf-8')
    settings['active_environment'] = env_doc['id']
    settings.setdefault('mcp', {})['http_port'] = MCP_PORT
    settings_path.write_text(json.dumps(settings, indent=2), encoding='utf-8')
    server = subprocess.Popen([str(CLI), 'serve', '--browser', '--port', str(API_PORT), '--workspace', str(ws)],
                              env=env, stderr=subprocess.DEVNULL)
    for _ in range(100):
        if port_open(API_PORT) and port_open(MCP_PORT):
            break
        time.sleep(0.1)
    html = urllib.request.urlopen(f'http://127.0.0.1:{API_PORT}/').read().decode()
    token = json.loads(re.search(r'window\.__RUNPOST_BACKEND__=(.*?);</script>', html).group(1))['token']
    config = {'mcpServers': {'runpost': {'type': 'http', 'url': f'http://127.0.0.1:{MCP_PORT}/mcp',
                                         'headers': {'X-RunPost-Token': token}}}}
    return server, token, config


def agent_meter(token):
    request = urllib.request.Request(f'http://127.0.0.1:{API_PORT}/api/agent', headers={'X-RunPost-Token': token})
    state = json.loads(urllib.request.urlopen(request).read())
    return {'meter': state['meter'],
            'calls': [{k: c[k] for k in ('tool', 'target', 'status', 'raw', 'sent')} for c in state['calls']]}


def run_arm(task, arm, model, dry):
    if dry:
        project = pathlib.Path(tempfile.gettempdir()) / f'runpost-bench-{task}-{arm}-XXXX'
    else:
        project = pathlib.Path(tempfile.mkdtemp(prefix=f'runpost-bench-{task}-{arm}-'))
        shutil.copy(COLLECTION, project / COLLECTION.name)
    server = token = None
    config = {'mcpServers': {}}
    if arm != 'curl' and not dry:
        server, token, config = prepare_runpost(project)
    config_path = project / 'mcp.json'
    if not dry:
        config_path.write_text(json.dumps(config), encoding='utf-8')
    allowed = ['Read', 'Bash(curl:*)'] if arm == 'curl' else ['Read', 'mcp__runpost']
    command = ['claude', '-p', prompt(task, arm), '--output-format', 'json', '--max-turns', '40',
               '--strict-mcp-config', '--mcp-config', str(config_path),
               '--setting-sources', 'project,local', '--no-session-persistence',
               '--allowedTools', *allowed]
    if arm == 'runpost-skill':
        command += ['--plugin-dir', str(PLUGIN)]
    if model:
        command += ['--model', model]
    print(f'\n== {task} · {arm}: {ARMS[arm]}\n   cwd={project}\n   ' +
          ' '.join(c if ' ' not in c else '"…prompt…"' for c in command))
    if dry:
        return None
    started = time.time()
    try:
        out = subprocess.run(command, cwd=project, capture_output=True, text=True, encoding='utf-8', timeout=1800)
        result = json.loads(out.stdout) if out.stdout.strip().startswith('{') else {'error': out.stdout[-2000:] + out.stderr[-2000:]}
        meter = agent_meter(token) if server else None
    finally:
        if server:
            server.terminate()
            server.wait(timeout=10)
    usage = result.get('usage', {})
    row = {
        'task': task, 'arm': arm, 'seconds': round(time.time() - started),
        'turns': result.get('num_turns'), 'cost_usd': result.get('total_cost_usd'),
        'input': usage.get('input_tokens', 0), 'cache_write': usage.get('cache_creation_input_tokens', 0),
        'cache_read': usage.get('cache_read_input_tokens', 0), 'output': usage.get('output_tokens', 0),
        'is_error': result.get('is_error', 'error' in result), 'answer': (result.get('result') or '')[:4000],
        'meter': meter,
    }
    row['raw'] = row['input'] + row['cache_write'] + row['cache_read'] + row['output']
    row['effective'] = row['input'] + row['cache_write'] + row['output'] + round(row['cache_read'] / 10)
    return row


def markdown(rows):
    lines = ['| Tarea | Brazo | Turnos | Entrada | Escritura caché | Lectura caché | Salida | Tokens en bruto | Tokens efectivos | Coste (USD) |',
             '|---|---|---:|---:|---:|---:|---:|---:|---:|---:|']
    for r in rows:
        lines.append(f"| {r['task']} | {r['arm']} | {r['turns']} | {r['input']} | {r['cache_write']} | {r['cache_read']} | "
                     f"{r['output']} | {r['raw']} | {r['effective']} | {r['cost_usd']} |")
    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tasks', default='t1,t2')
    parser.add_argument('--arms', default='curl,runpost,runpost-skill')
    parser.add_argument('--model', default=None)
    parser.add_argument('--runs', type=int, default=1)
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    if not args.dry_run and not CLI.exists():
        sys.exit(f'falta {CLI}: cargo build --release -p runpost-cli')
    mock = ensure_mock(args.dry_run)
    rows = []
    try:
        for _ in range(args.runs):
            for task in args.tasks.split(','):
                for arm in args.arms.split(','):
                    row = run_arm(task, arm, args.model, args.dry_run)
                    if row:
                        rows.append(row)
                        print(markdown([row]).splitlines()[-1])
    finally:
        if mock:
            mock.terminate()
    if rows:
        stamp = datetime.datetime.now().strftime('%Y%m%d-%H%M')
        out = ROOT / 'docs' / 'bench' / f'run-{stamp}.json'
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(rows, ensure_ascii=False, indent=2), encoding='utf-8')
        print('\n' + markdown(rows) + f'\n\nResultados: {out.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
