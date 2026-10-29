from copy import deepcopy

import pytest

import update_mcp_licenses as licenses


def graph(tmp_path, names):
    packages = []
    for name in names:
        directory = tmp_path / name
        directory.mkdir(exist_ok=True)
        (directory / 'LICENSE').write_text(f'Complete {name} upstream license\n', encoding='utf-8')
        packages.append(dict(id=name, name=name, version='1.0', license='MIT',
                             manifest_path=str(directory / 'Cargo.toml')))
    nodes = [dict(id='app', deps=[dict(name='rmcp', pkg='common')])]
    nodes.extend(dict(id=name, deps=[dict(name=other, pkg=other) for other in names if other != 'common']
                      if name == 'common' else []) for name in names)
    return dict(packages=packages, resolve=dict(root='app', nodes=nodes))


def test_arm_and_x86_generate_identical_complete_notice_files(tmp_path, monkeypatch):
    full = graph(tmp_path, ['common', 'x86-only'])
    arm = graph(tmp_path, ['common'])
    monkeypatch.setattr(licenses, 'PACKAGES', {('common', '1.0'), ('x86-only', '1.0')})
    calls = []
    def metadata(features, host):
        assert features == 'desktop,mcp'
        calls.append(host)
        return arm if host == 'aarch64-apple-darwin' else full
    monkeypatch.setattr(licenses, 'metadata', metadata)
    windows = licenses.render('x86_64-pc-windows-msvc')
    assert licenses.render('aarch64-apple-darwin') == windows
    assert licenses.render('x86_64-apple-darwin') == windows
    assert 'Complete x86-only upstream license' in windows
    assert calls == ['x86_64-pc-windows-msvc', 'aarch64-apple-darwin',
                     'x86_64-pc-windows-msvc', 'x86_64-apple-darwin']


def test_active_unreviewed_dependency_is_not_hidden_by_canonical_sources(tmp_path, monkeypatch):
    full = graph(tmp_path, ['common', 'unreviewed'])
    monkeypatch.setattr(licenses, 'PACKAGES', {('common', '1.0')})
    monkeypatch.setattr(licenses, 'metadata', lambda *_: full)
    with pytest.raises(ValueError, match='license review required.*unreviewed'):
        licenses.render('aarch64-apple-darwin')


def test_unavailable_canonical_source_is_an_explicit_error(tmp_path, monkeypatch):
    full = graph(tmp_path, ['common'])
    monkeypatch.setattr(licenses, 'PACKAGES', {('common', '1.0'), ('x86-only', '1.0')})
    monkeypatch.setattr(licenses, 'metadata', lambda *_: deepcopy(full))
    with pytest.raises(ValueError, match='Missing MCP license sources.*x86-only.*cargo fetch'):
        licenses.render('aarch64-apple-darwin')
