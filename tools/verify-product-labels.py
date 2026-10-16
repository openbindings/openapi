"""Keep implementation and review campaign labels out of tracked product files."""
from pathlib import Path
import argparse
import re
import subprocess
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SLICE = r'S(?:10|[1-9])(?:[a-f]{1,2}(?:/[a-f])?)?'
PATTERNS = [
    # Bare numbered tokens also name upstream products. Require record syntax
    # for those; suffixed units and identifier/path prefixes are distinctive.
    re.compile(r'\b(?:slice|phase|unit)[\s_/-]+(?:' + SLICE + r'|[VX])\b', re.I),
    re.compile(r'^\s*(?:(?:#{1,6}|//|/\*|\*|[-+])\s*)?' + SLICE + r'\s*(?::|—|–|\s-)(?:\s|$)', re.I),
    re.compile(r'(?<![A-Za-z0-9])S(?:10|[1-9])[a-fA-F]{1,2}(?:/[a-fA-F])?(?![A-Za-z0-9])'),
    re.compile(r'(?<![A-Za-z0-9])s(?:10|[1-9])[a-f]{0,2}[_/-](?=[A-Za-z])'),
    re.compile(r'(?<![A-Za-z0-9])(?:CR|EV|API|HD|RO|N|PKG|OAC|HYG|LABEL|MSRV)-\d+[a-z]?\b'),
    re.compile(r'(?<![A-Za-z0-9])(?:ro|cr|ev|hd|api|oac|pkg|n)\d+_'),
    re.compile(r'\breview_(?:[sf]\d+_)+', re.I),
    re.compile(r'\bD-[A-Z][A-Z0-9-]*\b'),
    re.compile(r'\bV[-_]map\b', re.I),
    # Quoted historical labels remain forbidden in maintained product text.
    re.compile(r'\b[\w-]+-loop-\d{4}-\d{2}-\d{2}\b|\b(?:role|role-contract)-loop\b'),
    re.compile(r'\bround[-_]0?\d+\b|\bloop[-_][A-Z]\d*\b', re.I),
    re.compile(r'\brounds/\d+/(?:oracle|design|authors)/', re.I),
    re.compile(r'\bAmendment\s+\d+\b'),
]
SECTION_CITATION = re.compile(r'\b(?:Annex|Appendix)\s+D-[A-Z0-9-]+\b')


def matches(text):
    # Preserve named sections in upstream standards, including lettered annexes.
    text = SECTION_CITATION.sub('', text)
    return any(pattern.search(text) for pattern in PATTERNS)


def violations(name, content):
    result = [f'{name}: filename contains a campaign label'] if (
        matches(name) or re.search(r'(?:^|/)' + SLICE + r'(?:[_.-]|$)', name, re.I)
    ) else []
    for number, line in enumerate(content.splitlines(), 1):
        # Reconstruction patches retain upstream identifiers in removed/context
        # lines; only additions are maintained product content.
        if name.endswith('.patch') and (not line.startswith('+') or line.startswith('+++')):
            continue
        if matches(line):
            result.append(f'{name}:{number}: {line.strip()}')
    return result


def product_files(root=ROOT, tracked_files=None):
    if tracked_files is not None:
        names = tracked_files.read_bytes()
    elif (root / '.git').exists():
        names = subprocess.check_output(['git', 'ls-files', '--cached', '-z'], cwd=root)
    else:
        raise SystemExit('Source archives require --tracked-files pointing to a git ls-files -z manifest')
    paths = set()
    for name in names.decode('utf-8').split('\0'):
        if not name:
            continue
        path = Path(name)
        if path.is_absolute() or '..' in path.parts:
            raise SystemExit(f'Tracked-file manifest contains an unsafe path: {name!r}')
        if (root / path).is_file():
            paths.add(root / path)
    return sorted(paths)


def self_test():
    labels = [*('slice S' + str(i) for i in range(1, 11)),
              'S' + '6e', 's' + '6f_resolver', 's' + '6de_review', 'S' + '6DE', 's' + '6c/fixture',
              'S' + '10c', 's' + '10c_test', 'S' + '8a', 'S' + '6f', 'S' + '6d/e',
              *(prefix + '-3' for prefix in ['CR', 'EV', 'API', 'HD', 'RO', 'N', 'PKG', 'OAC', 'HYG', 'LABEL', 'MSRV']),
              'review_' + 's09_anonymous_scope', 'review_' + 's11_same_uri',
              'review_' + 'f01_f06_preparation', 'review_' + 'f04_diagnostics',
              'ro' + '18_test', 'cr' + '2_test', 'D' + '-METASCHEMA', 'D' + '-DISCOVERY-HOSTS',
              'slice ' + 'V', 'phase ' + 'X', 'V' + '-map', 'S3' + ': document checks',
              'sdk-maturity' + '-loop-2026-10-10', 'round' + '-01',
              'Amendment ' + '1', 'rounds/' + '02/oracle/accepted/case.json',
              'rounds/' + '02/design/oneof-acceptance/cases.json']
    for label in labels:
        assert violations('src/check.rs', '// ' + label), label
    for allowed in ['OBI-01 through OBI-13', 'RFC 3986 §5.2', 'JSON Schema §9.4.2',
                    'DISC-C-02', 'JSON Schema test suite draft2020-12', 'AWS S3',
                    'AWS S3: object storage',
                    'round 1 of the fixed-point algorithm', 'HTTP/2 RFC 9113 Appendix D-1',
                    'ISO/IEC 9899 Annex D-UNIVERSAL', 'RFC 8446 section D.4',
                    'ES2022', 'oneOf(U,V)', 'impl<V> Value<V>', 'X-Request-ID',
                    'infinite-loop-detection.json', 'round-trip', 'let s1 = "x";']:
        assert not violations('docs/subject.md', allowed), allowed
    label = 'S' + '6e'
    assert violations('tools/qualification/inputs/spec/upstream.md', label)
    for name in ['s3', 'S10', 'unit-' + 'v', 'slice-' + 'x']:
        assert violations('tests/' + name + '.rs', '')
    for name in ['x', 'v', 'x_axis']:
        assert not violations('src/' + name + '.rs', '')
    for name in ['s' + '6f_resolver', 'review_' + 'f01_f06_preparation', 'round' + '-01']:
        assert violations('tests/' + name + '.rs', '')
    assert violations('CHANGELOG.md', label)
    assert violations('docs/migration.md', label)
    assert violations('docs/new.md', label)
    assert not violations('docs/upstream.patch', '-' + label)
    assert violations('docs/upstream.patch', '+' + label)
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / '.git').mkdir()
        (root / 'tracked.md').write_text('AWS S3')
        (root / 'untracked.md').write_text(label)
        generated = root / 'tools/qualification/corpus/run/full-report.json'
        generated.parent.mkdir(parents=True)
        generated.write_text(label)
        names = b'tracked.md\0deleted.md\0'
        with patch('subprocess.check_output', return_value=names) as git:
            assert product_files(root) == [root / 'tracked.md']
            git.assert_called_once_with(['git', 'ls-files', '--cached', '-z'], cwd=root)
        (root / '.git').rmdir()
        manifest = root / 'tracked-files'
        manifest.write_bytes(names)
        assert product_files(root, manifest) == [root / 'tracked.md']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--tracked-files', type=Path, help='NUL-separated git ls-files list for a source archive')
    args = parser.parse_args()
    if args.self_test:
        self_test()
    errors = []
    for path in product_files(tracked_files=args.tracked_files):
        try:
            content = path.read_text(encoding='utf-8')
        except UnicodeError:
            content = ''
        errors.extend(violations(path.relative_to(ROOT).as_posix(), content))
    if errors:
        raise SystemExit('\n'.join(errors))
    print('Product labels: tracked filenames and content passed')


if __name__ == '__main__':
    main()
