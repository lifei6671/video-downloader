import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location(
    'manual_runner', Path(__file__).resolve().parents[1] / 'scripts/run_m1.py')
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


@pytest.mark.parametrize('network,expected', [([], 'socks5://127.0.0.1:3067'),
    (['--proxy', 'http://127.0.0.1:3067'], 'http://127.0.0.1:3067'),
    (['--direct'], None)])
def test_manual_credentials_stdin_and_network(tmp_path, monkeypatch, capsys, network, expected):
    cookie = tmp_path / 'cookie.txt'
    cookie.write_text('session=private-value')
    command = ['run_m1.py', '--cookie-file', str(cookie), '--probe', sys.executable,
               '--output-dir', str(tmp_path / 'download'), *network]
    monkeypatch.setattr(sys, 'argv', command)
    calls = []
    def run(args, *, input):
        calls.append((args, json.loads(input)))
        return subprocess.CompletedProcess(args, 1)
    monkeypatch.setattr(runner.subprocess, 'run', run)
    assert runner.main() == 1
    args, config = calls[0]
    assert config['profiles']['tiktok']['cookie'] == 'session=private-value'
    assert config['profiles']['tiktok']['proxy'] == expected
    assert 'private-value' not in repr(args) + capsys.readouterr().out
    assert Path(config['output_dir']).is_dir()


def test_existing_config_mode(tmp_path, monkeypatch):
    config = tmp_path / 'config.json'
    config.write_text(json.dumps({'inputs':['example'], 'profiles':{},
                                  'output_dir':str(tmp_path / 'out')}))
    monkeypatch.setattr(sys, 'argv', ['run_m1.py', str(config), '--probe', sys.executable,
                                     '--engine', sys.executable])
    def run(args, *, input):
        assert len(args) == 2
        assert json.loads(input)['inputs'] == ['example']
        return subprocess.CompletedProcess(args, 0)
    monkeypatch.setattr(runner.subprocess, 'run', run)
    assert runner.main() == 0


def test_missing_cookie_has_no_traceback(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(sys, 'argv', ['run_m1.py', '--probe', sys.executable,
        '--cookie-file', str(tmp_path/'missing.txt')])
    with pytest.raises(SystemExit) as error:
        runner.main()
    assert error.value.code == 2
    assert 'Traceback' not in capsys.readouterr().err
