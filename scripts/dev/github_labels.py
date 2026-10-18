# Crea o actualiza en GitHub las etiquetas de .github/labels.yml (good first issue, help wanted...).
# Usa el CLI `gh` ya autenticado. No es parte del producto; se ejecuta una vez por repositorio.
# Uso: python scripts/dev/github_labels.py <owner/repo> [--dry-run]
import pathlib
import subprocess
import sys


def labels(path):
    """Lee el YAML simple de labels.yml (lista de name/color/description) sin dependencias."""
    items, current = [], None
    for raw in path.read_text(encoding='utf-8').splitlines():
        line = raw.strip()
        if not line or line.startswith('#'):
            continue
        if line.startswith('- '):
            current = {}
            items.append(current)
            line = line[2:]
        key, _, value = line.partition(':')
        current[key.strip()] = value.strip().strip('"')
    return items


def main():
    if len(sys.argv) < 2:
        sys.exit('uso: python scripts/dev/github_labels.py <owner/repo> [--dry-run]')
    repo, dry = sys.argv[1], '--dry-run' in sys.argv
    root = pathlib.Path(__file__).resolve().parents[2]
    for label in labels(root / '.github' / 'labels.yml'):
        command = ['gh', 'label', 'create', label['name'], '--repo', repo, '--force',
                   '--color', label['color'], '--description', label['description']]
        print(' '.join(command))
        if not dry:
            subprocess.run(command, check=True)


if __name__ == '__main__':
    main()
