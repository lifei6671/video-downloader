import gzip
import hashlib
import io
import sys
import zipfile
from pathlib import Path

import pytest

import engine
import local_tools
import runtime_assets
import runtime_dependencies as deps


@pytest.fixture
def isolated(monkeypatch, tmp_path):
    monkeypatch.setattr(local_tools, 'MANAGED_DIR', tmp_path)
    monkeypatch.setattr(local_tools.shutil, 'which', lambda _: None)
    monkeypatch.setattr(local_tools, 'search_paths', lambda _: [])
    def version(path, flag):
        body = Path(path).read_bytes()
        return body.decode() if body.startswith((b'ffmpeg version', b'ffprobe version', b'deno ')) else ''
    monkeypatch.setattr(local_tools, 'tool_output', version)
    return tmp_path


def fixture_downloads(monkeypatch, tool, failure=None):
    suffix = '.exe' if sys.platform == 'win32' else ''
    names = ['ffmpeg', 'ffprobe'] if tool == 'ffmpeg' else ['deno']
    downloads, bodies, requests = [], {}, []
    for name in names:
        filename = name + suffix
        binary = f'{name} version 6.1.1\n'.encode() if name != 'deno' else b'deno 2.9.7\n'
        if name == 'deno':
            stream = io.BytesIO()
            with zipfile.ZipFile(stream, 'w') as archive:
                archive.writestr(filename, binary)
                archive.writestr('../escaped', 'must never be extracted')
            body, compression = stream.getvalue(), 'zip'
        else:
            body, compression = gzip.compress(binary), 'gzip'
        url = f'https://fixture.test/{name}'
        downloads.append((url, filename, compression, len(body), hashlib.sha256(body).hexdigest()))
        bodies[url] = body
    license_body = b'fixture-license'
    downloads.append(('https://fixture.test/license', 'LICENSE', 'raw', len(license_body), hashlib.sha256(license_body).hexdigest()))
    bodies[downloads[-1][0]] = license_body
    monkeypatch.setattr(deps, 'assets', lambda kind, _: downloads if kind == tool else [])
    class Response(io.BytesIO):
        status = 200
    class Opener:
        def open(self, request, timeout):
            requests.append(request)
            assert not any(name.lower() in ('cookie', 'authorization') for name in request.headers)
            if failure == 'timeout':
                raise TimeoutError()
            body = bodies[request.full_url]
            return Response(body[:-1] if failure == 'truncated' else body + b'x' if failure == 'oversized' else body)
    monkeypatch.setattr(deps, 'build_opener', lambda *_: Opener())
    return requests


@pytest.mark.parametrize('tool', ['ffmpeg', 'javascript'])
def test_install_checks_hash_executes_version_and_becomes_real_resolver_candidate(isolated, monkeypatch, tool):
    requests = fixture_downloads(monkeypatch, tool)
    stage = isolated / '.install-fixture'
    stage.mkdir()
    progress = []
    result = deps.install(tool, stage, lambda event, **value: progress.append(value))
    row = next(row for row in result['tools'] if row['id'] == tool)
    assert row['available'] and row['source'] == 'managed'
    assert Path(row['path']).parent == isolated / tool
    assert (isolated / tool / 'LICENSE').is_file()
    assert not (isolated / 'escaped').exists()
    assert progress[-1]['downloaded_bytes'] == progress[-1]['total_bytes']
    assert [p['downloaded_bytes'] for p in progress] == sorted(p['downloaded_bytes'] for p in progress)
    assert requests
    if tool == 'javascript':
        assert local_tools.javascript_runtime()['deno']['path'] == row['path']
    else:
        assert local_tools.installed_tool('ffmpeg') == row['path']
        assert local_tools.installed_tool('ffprobe')


@pytest.mark.parametrize('failure,code', [('truncated', 'DEPENDENCY_CHECKSUM_MISMATCH'),
                                       ('oversized', 'DEPENDENCY_CHECKSUM_MISMATCH'),
                                       ('timeout', 'DEPENDENCY_TIMEOUT')])
def test_download_failure_never_publishes_partial_binary(isolated, monkeypatch, failure, code):
    fixture_downloads(monkeypatch, 'ffmpeg', failure)
    stage = isolated / '.install-fixture'
    stage.mkdir()
    broken = isolated / 'ffmpeg'
    broken.mkdir()
    (broken / 'keep').write_text('existing broken installation')
    with pytest.raises(deps.DependencyError, match=code):
        deps.install('ffmpeg', stage, lambda *_args, **_kwargs: None)
    assert list(broken.iterdir()) == [broken / 'keep']
    assert not deps.status()['tools'][0]['available']


def test_invalid_executable_is_not_published(isolated, monkeypatch):
    fixture_downloads(monkeypatch, 'javascript')
    monkeypatch.setattr(local_tools, 'tool_output', lambda *_: 'deno 2.2.0')
    stage = isolated / '.install-fixture'
    stage.mkdir()
    with pytest.raises(deps.DependencyError, match='DEPENDENCY_VALIDATION_FAILED'):
        deps.install('javascript', stage, lambda *_args, **_kwargs: None)
    assert not (isolated / 'javascript').exists()


def test_existing_system_tool_and_supported_node_prevent_download(isolated, monkeypatch):
    node = isolated / 'system-node'
    node.write_bytes(b'node')
    monkeypatch.setattr(local_tools, 'javascript_runtime', lambda: {'node': {'path': str(node)}})
    monkeypatch.setattr(local_tools, 'tool_output', lambda *_: 'v22.0.0')
    monkeypatch.setattr(deps, 'build_opener', lambda *_: pytest.fail('must not download'))
    assert deps.install('javascript', isolated / 'not-needed', lambda *_args, **_kwargs: None)['tools'][1]['available']


def test_engine_dependency_command_uses_same_managed_root_without_platform_request(isolated, monkeypatch):
    monkeypatch.setattr(engine, 'parse_input', lambda *_: pytest.fail('not a media request'))
    events = []
    engine.operate('check_dependencies', {'runtime_directory': str(isolated)}, lambda name, **data: events.append((name, data)))
    assert events[0][0] == 'dependencies'
    assert events[0][1]['result']['directory'] == str(isolated)
    assert not any(row['available'] for row in events[0][1]['result']['tools'])


@pytest.mark.parametrize('platform', ['win32-x64', 'darwin-x64', 'darwin-arm64'])
@pytest.mark.parametrize('tool', ['ffmpeg', 'javascript'])
def test_pinned_downloads_cover_supported_targets_and_include_license(tool, platform):
    assets = runtime_assets.assets(tool, platform)
    assert assets and any(item[1] == 'LICENSE' for item in assets)
    assert all(url.startswith('https://') and len(digest) == 64 and size > 0 for url, _, _, size, digest in assets)
    assert runtime_assets.assets(tool, 'win32-arm64') == []


def test_redirect_boundary_rejects_other_origins():
    from urllib.request import Request
    request = Request('https://github.com/source')
    redirects = deps.DownloadRedirects()
    for target in ['http://github.com/file', 'https://evil.test/file', 'https://github.com:444/file']:
        with pytest.raises(deps.DependencyError, match='DEPENDENCY_NETWORK_ERROR'):
            redirects.redirect_request(request, None, 302, '', {}, target)
