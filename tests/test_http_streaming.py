"""Real pinned HttpFD through GuardedRH; all media comes from the local proxy fixture."""
import json
import time

import pytest
from yt_dlp import YoutubeDL

import engine


@pytest.mark.parametrize('chunk', [None, '0', '10485760'])
@pytest.mark.parametrize('mode,size', [('good', 33), ('range', 10 * 1024 * 1024 + 12345),
    ('ignore', 10 * 1024 * 1024 + 12345), ('range', 1024), ('unknown', 1024),
    ('transient', 4096), ('interrupted', 8192), ('range_chunked', 10 * 1024 * 1024 + 12345),
    ('range_zero_length', 10 * 1024 * 1024 + 12345)])
def test_native_http_integrity_modes(monkeypatch, tmp_path, media_proxy, media_bodies, chunk, mode, size):
    if chunk is None:
        monkeypatch.delenv('YINGQU_HTTP_CHUNK_SIZE', raising=False)
    else:
        monkeypatch.setenv('YINGQU_HTTP_CHUNK_SIZE', chunk)
    body = b'\x00\x00\x00\x18ftypmp42' + b'x' * size
    media_bodies[mode] = body
    proxy, requests, _, _ = media_proxy
    def extract(self, url, download):
        return self.process_ie_result(dict(id='123', webpage_url=url, title='fixture',
            extractor='fixture', extractor_key='Fixture', formats=[dict(format_id='selected',
                ext='mp4', protocol='http', vcodec='h264', acodec='aac',
                downloader_options={'http_chunk_size': 10 * 1024 * 1024},
                url=f'http://v.fixture.tiktokcdn.com/{mode}.mp4?signature=private')]), download=False)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    events = []
    started = time.perf_counter()
    engine.operate('download_video', dict(input='https://www.tiktok.com/@u/video/123',
        profiles={'tiktok': {'proxy': proxy, 'cookie': 'session=private'}},
        expected={'platform': 'tiktok', 'video_id': '123'}, staging_dir=str(tmp_path)),
        lambda kind, **data: events.append((kind, data)))
    elapsed = time.perf_counter() - started
    assert (tmp_path / 'video.mp4').read_bytes() == body
    assert list(tmp_path.iterdir()) == [tmp_path / 'video.mp4']
    counts = [data['downloaded_bytes'] for kind, data in events if kind == 'progress']
    assert counts == sorted(counts) and counts[-1] == len(body)
    assert events[-1][0] == 'download_finished' and events[-1][1]['bytes'] == len(body)
    assert 'private' not in json.dumps(events) and 'tiktokcdn.com' not in json.dumps(events)
    assert not any('Cookie' in headers for _, headers in requests)
    if chunk != '0':
        assert all('Range' in headers for _, headers in requests)
    elif mode != 'interrupted':
        assert all('Range' not in headers for _, headers in requests)
    if mode in ('range', 'range_chunked', 'range_zero_length') and size > int(chunk or '10485760') > 0:
        assert len(requests) >= 2
    if mode == 'transient':
        assert len(requests) == 2
    print(json.dumps(dict(mode=mode, chunk=int(chunk) if chunk else 'inherited', bytes=len(body), requests=len(requests), seconds=round(elapsed, 4))))


@pytest.mark.parametrize('chunk', ['0', '10485760'])
@pytest.mark.parametrize('mode,code,attempts', [('status401', 'ACCESS_DENIED', 1),
    ('status403', 'ACCESS_DENIED', 1), ('status404', 'NOT_FOUND', 1),
    ('status429', 'RISK_CONTROL', 1), ('status503', 'NETWORK_ERROR', 2),
    ('bad_range', 'INCOMPLETE_MEDIA', 2), ('truncated', 'INCOMPLETE_MEDIA', 2)])
def test_native_http_failure_retry_limits(monkeypatch, tmp_path, media_proxy, chunk, mode, code, attempts):
    monkeypatch.setenv('YINGQU_HTTP_CHUNK_SIZE', chunk)
    proxy, requests, _, _ = media_proxy
    def extract(self, url, download):
        return self.process_ie_result(dict(id='123', webpage_url=url, title='fixture',
            extractor='fixture', extractor_key='Fixture', formats=[dict(format_id='selected',
                ext='mp4', protocol='http', vcodec='h264', acodec='aac',
                url=f'http://v.fixture.tiktokcdn.com/{mode}.mp4?signature=private')]), download=False)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    events = []
    with pytest.raises(engine.EngineError) as caught:
        engine.operate('download_video', dict(input='https://www.tiktok.com/@u/video/123',
            profiles={'tiktok': {'proxy': proxy}}, expected={'platform': 'tiktok', 'video_id': '123'},
            staging_dir=str(tmp_path)), lambda kind, **data: events.append((kind, data)))
    assert caught.value.code == code
    assert len(requests) == attempts
    assert not any(kind == 'download_finished' for kind, _ in events)
    assert not (tmp_path / 'video.mp4').exists()


def test_chunked_range_early_eof_has_bounded_retries(monkeypatch, tmp_path, media_proxy, media_bodies):
    monkeypatch.setenv('YINGQU_HTTP_CHUNK_SIZE', '10485760')
    mode = 'range_chunked_truncated'
    media_bodies[mode] = b'\x00\x00\x00\x18ftypmp42' + b'x' * 8192
    proxy, requests, _, _ = media_proxy
    def extract(self, url, download):
        return self.process_ie_result(dict(id='123', webpage_url=url, title='fixture',
            extractor='fixture', extractor_key='Fixture', formats=[dict(format_id='selected',
                ext='mp4', protocol='http', vcodec='h264', acodec='aac',
                url=f'http://v.fixture.tiktokcdn.com/{mode}.mp4?signature=private')]), download=False)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    events = []
    with pytest.raises(engine.EngineError) as caught:
        engine.operate('download_video', dict(input='https://www.tiktok.com/@u/video/123',
            profiles={'tiktok': {'proxy': proxy}}, expected={'platform': 'tiktok', 'video_id': '123'},
            staging_dir=str(tmp_path)), lambda kind, **data: events.append((kind, data)))
    assert caught.value.code == 'INCOMPLETE_MEDIA'
    assert len(requests) == 2 and all('Range' in headers for _, headers in requests)
    assert not any(kind == 'download_finished' for kind, _ in events)
    assert not (tmp_path / 'video.mp4').exists()


def test_chunk_option_is_bounded_and_has_conservative_default(monkeypatch):
    monkeypatch.delenv('YINGQU_HTTP_CHUNK_SIZE', raising=False)
    assert engine.http_chunk_size() == 0
    for value in ('-1', 'infinite', '10M', '999999999999'):
        monkeypatch.setenv('YINGQU_HTTP_CHUNK_SIZE', value)
        with pytest.raises(engine.EngineError, match='CONFIG_INVALID'):
            engine.http_chunk_size()


@pytest.mark.parametrize('size', [None, 0, 1048576, 10485760, 1073741824])
def test_youtube_saved_chunk_mode_overrides_legacy_environment(monkeypatch, size):
    monkeypatch.setenv('YINGQU_HTTP_CHUNK_SIZE', 'invalid-legacy-value')
    assert engine.http_chunk_size({'http_chunk_size': size}, 'youtube') == size


@pytest.mark.parametrize('size', [-1, 1, 1048577, 1074790400, True, 1048576.0, '10485760', {}])
def test_youtube_invalid_saved_chunk_size_is_rejected(size):
    with pytest.raises(engine.EngineError, match='CONFIG_INVALID'):
        engine.operate('resolve_video', {'input': 'https://youtu.be/BaW_jenozKc',
            'profiles': {'youtube': {'http_chunk_size': size}}}, lambda *_a, **_k: None)


@pytest.mark.parametrize('platform,url', [('douyin', 'https://www.douyin.com/video/123'),
    ('tiktok', 'https://www.tiktok.com/@u/video/123')])
def test_chunk_setting_cannot_spill_into_other_platforms(platform, url):
    with pytest.raises(engine.EngineError, match='CONFIG_INVALID'):
        engine.operate('resolve_video', {'input': url, 'profiles': {platform: {'http_chunk_size': 0}}},
                       lambda *_a, **_k: None)


@pytest.mark.parametrize('chunk', [None, 0, 1048576, 3 * 1048576])
def test_saved_youtube_modes_reach_real_http_downloader_without_changing_format(
        monkeypatch, tmp_path, media_proxy, media_bodies, chunk):
    from video_formats import public_formats
    monkeypatch.setenv('YINGQU_HTTP_CHUNK_SIZE', '10485760')
    monkeypatch.setattr(engine, 'javascript_runtime', lambda: {'node': {'path': 'fixture-node'}})
    body = b'\x00\x00\x00\x18ftypmp42' + b'x' * (3 * 1048576 + 123)
    media_bodies['range'] = body
    proxy, requests, _, _ = media_proxy
    format_info = dict(format_id='selected', ext='mp4', protocol='http', vcodec='h264', acodec='aac',
        downloader_options={'http_chunk_size': 2 * 1048576},
        url='http://v.fixture.googlevideo.com/range.mp4?signature=private')
    selected = public_formats({'formats': [format_info]}, merge=True)[0]['format_id']
    def extract(self, url, download):
        return self.process_ie_result(dict(id='BaW_jenozKc', webpage_url=url, title='fixture',
            extractor='fixture', extractor_key='Fixture', formats=[format_info]), download=False)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    events = []
    engine.operate('download_video', dict(input='https://youtu.be/BaW_jenozKc',
        profiles={'youtube': {'proxy': proxy, 'http_chunk_size': chunk}}, format_id=selected,
        expected={'platform': 'youtube', 'video_id': 'BaW_jenozKc'}, staging_dir=str(tmp_path)),
        lambda kind, **data: events.append((kind, data)))
    assert (tmp_path / 'video.mp4').read_bytes() == body
    assert list(tmp_path.iterdir()) == [tmp_path / 'video.mp4']
    assert events[-1][0] == 'download_finished'
    assert events[-1][1]['format_id'] == selected
    assert events[-1][1]['bytes'] == events[-1][1]['transferred_bytes'] == len(body)
    if chunk == 0:
        assert len(requests) == 1 and 'Range' not in requests[0][1]
    else:
        # HttpFD jitters the actual chunk length by up to 5 percent.
        first_end = int(requests[0][1]['Range'].split('-')[1])
        expected_chunk = 2 * 1048576 if chunk is None else chunk
        assert expected_chunk * .95 <= first_end + 1 <= expected_chunk
        assert len(requests) >= 2
    assert 'private' not in json.dumps(events) and 'googlevideo.com' not in json.dumps(events)
