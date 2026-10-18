import http.server
import runpy
import shutil
import socket
import sys
from pathlib import Path

import pytest


def test_http_fixture_listens_without_reverse_dns(tmp_path, monkeypatch):
    fixture = tmp_path / 'tdl_mock.py'
    shutil.copy2(Path(__file__).parent / 'fixtures/tdl_mock.py', fixture)
    (tmp_path / 'options.json').write_text('{}', encoding='utf-8')
    monkeypatch.setattr(sys, 'argv', [str(fixture), 'dl', '-u',
                                    'https://t.me/c/100/9', '--serve', '--port', '0'])

    def unexpected_lookup(*_):
        pytest.fail('loopback HTTP fixture must not perform reverse DNS')

    listening = []

    def serve(server):
        try:
            assert server.server_address[0] == '127.0.0.1'
            with socket.create_connection(server.server_address, timeout=1):
                pass
            listening.append(server.server_address[1])
        finally:
            server.server_close()

    monkeypatch.setattr(socket, 'getfqdn', unexpected_lookup)
    monkeypatch.setattr(http.server.ThreadingHTTPServer, 'serve_forever', serve)
    runpy.run_path(str(fixture), run_name='__main__')
    assert listening and listening[0] > 0
