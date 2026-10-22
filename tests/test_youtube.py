import json
import sys
from http.cookiejar import CookieJar
from pathlib import Path

import pytest
from yt_dlp import YoutubeDL

import browser_cookies
import engine
import local_tools
from network import TargetError, validate_target
from video_formats import public_formats, select_format


URL = 'https://www.youtube.com/watch?v=BaW_jenozKc'


@pytest.mark.parametrize('video_id', ['Ql7DR8YKwSg', 'y300HjYnHPw', 'a6cnYFcd7JQ'])
def test_reported_youtube_link_is_supported_without_network_for_identity(video_id):
    events = []
    url = 'https://www.youtube.com/watch?v=' + video_id
    engine.operate('identify_input', {'input': url}, lambda kind, **data: events.append(data))
    assert events[0]['result']['platform'] == 'youtube'
    assert events[0]['result']['video_id'] == video_id


@pytest.mark.parametrize('url', [URL + '&list=ignored&tracking=x',
    'https://youtu.be/BaW_jenozKc?t=30', 'https://m.youtube.com/shorts/BaW_jenozKc',
    'https://youtube.com/embed/BaW_jenozKc', 'https://youtube.com/shorts/BaW_jenozKc/',
    'https://youtube.com/embed/BaW_jenozKc/'])
def test_youtube_single_identity(url):
    assert engine.parse_input(url) == ('youtube', URL, False)
    identity = engine.input_identity('youtube', URL, 'auto', URL)
    assert identity['video_id'] == 'BaW_jenozKc' and identity['resolver'] == 'yt-dlp'
    with pytest.raises(engine.EngineError, match='UNSUPPORTED_CONTENT'):
        engine.input_identity('youtube', URL, 'dtk', URL)


@pytest.mark.parametrize('url', ['https://youtube.com/playlist?list=123',
    'https://youtube.com/watch?v=../../foo', 'https://youtu.be/short',
    'https://youtube.com.evil.org/watch?v=BaW_jenozKc',
    'https://youtube.com/@channel', 'https://youtube.com/live/BaW_jenozKc'])
def test_rejects_non_video_links(url):
    with pytest.raises(engine.EngineError):
        engine.parse_input(url)


def formats():
    return [dict(format_id='18', ext='mp4', vcodec='avc1', acodec='aac', width=640,
                 height=360, protocol='https', url='https://v.googlevideo.com/combined'),
            dict(format_id='140', ext='m4a', vcodec='none', acodec='aac', filesize=20,
                 protocol='https', url='https://v.googlevideo.com/audio?secret=private'),
            dict(format_id='137', ext='mp4', vcodec='avc1', acodec='none', width=1920,
                 height=1080, filesize=100, protocol='https', url='https://v.googlevideo.com/video')]


def test_hd_selection_pairs_audio_without_exposing_urls_or_downgrading():
    context = {'formats': formats()}
    public = public_formats(context, merge=True)
    assert len(public) == 2 and public[0]['height'] == 1080
    assert public[0]['requires_merge'] and public[0]['filesize'] == 120
    assert not public[1]['requires_merge']
    assert 'secret' not in json.dumps(public) and 'googlevideo' not in json.dumps(public)
    selected = next(select_format(context, public[0]['format_id'], merge=True))
    assert [f['format_id'] for f in selected['requested_formats']] == ['137', '140']
    assert selected['protocol'] == 'https+https'
    with pytest.raises(engine.EngineError, match='FORMAT_UNAVAILABLE'):
        next(select_format({'formats': formats()[:1]}, public[0]['format_id'], merge=True))


def test_hd_size_unknown_and_missing_audio_not_silent():
    items = formats()
    items[1]['filesize'] = None
    assert public_formats({'formats': items}, True)[0]['filesize'] is None
    with pytest.raises(engine.EngineError, match='UNSUPPORTED_CONTENT'):
        next(select_format({'formats': [items[-1]]}, merge=True))


@pytest.mark.parametrize('reverse', [False, True])
@pytest.mark.parametrize('chunk', ['missing', None, 0, 3 * 1048576])
def test_top_level_language_reaches_real_ytdlp_selector_and_public_choices(monkeypatch, tmp_path, reverse, chunk):
    monkeypatch.setattr(engine, 'javascript_runtime', lambda: {'node': {'path': 'fixture-node'}})
    items = [{**formats()[-1], 'language': None}, {**formats()[1], 'format_id': '140-ja', 'language': 'ja', 'abr': 100},
             {**formats()[1], 'format_id': '140-en', 'language': 'en', 'abr': 200}]
    for item in items:
        item['downloader_options'] = {'http_chunk_size': 10 * 1048576}
    def extract(self, url, download):
        info = dict(id='BaW_jenozKc', webpage_url=URL, title='fixture', language='ja',
            extractor='fixture', extractor_key='Fixture', formats=items[::-1] if reverse else items)
        result = self.process_ie_result(info, download=False)
        assert [f['format_id'] for f in result['requested_formats']] == ['137', '140-ja']
        return result
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    events = []
    engine.operate('resolve_video', {'input': URL}, lambda kind, **data: events.append(data))
    public = events[-1]['result']
    assert public['selected_format_id'] in {f['format_id'] for f in public['formats']}
    from yt_dlp.postprocessor.ffmpeg import FFmpegMergerPP
    monkeypatch.setattr(engine, 'installed_tool', lambda _: 'fixture-ffmpeg')
    monkeypatch.setattr(FFmpegMergerPP, 'available', property(lambda _: True))
    def download(self, info):
        assert [f['format_id'] for f in info['requested_formats']] == ['137', '140-ja']
        hook = self.params['progress_hooks'][0]
        for fmt in info['requested_formats']:
            assert fmt['downloader_options']['http_chunk_size'] == (10 * 1048576 if chunk in ('missing', None) else chunk)
            hook(dict(status='finished', info_dict=fmt, downloaded_bytes=fmt['filesize'], total_bytes=fmt['filesize']))
        (tmp_path / 'video.mp4').write_bytes(b'\x00\x00\x00\x18ftypmp42merged')
    monkeypatch.setattr(YoutubeDL, 'process_info', download)
    params = dict(input=URL, format_id=public['selected_format_id'], staging_dir=str(tmp_path),
        expected={'platform': 'youtube', 'video_id': 'BaW_jenozKc'})
    if chunk != 'missing':
        params['profiles'] = {'youtube': {'http_chunk_size': chunk}}
    engine.operate('download_video', params, lambda kind, **data: events.append(data))
    assert events[-1]['format_id'] == public['selected_format_id']
    items[:] = [fmt for fmt in items if fmt.get('language') != 'ja']
    empty = tmp_path / 'missing-audio'
    empty.mkdir()
    with pytest.raises(engine.EngineError, match='FORMAT_UNAVAILABLE'):
        engine.operate('download_video', {**params, 'staging_dir': str(empty)}, lambda *_a, **_k: None)
    assert list(empty.iterdir()) == []


@pytest.mark.parametrize('reverse', [False, True])
@pytest.mark.parametrize('metadata', ['preference', 'language', 'default', 'missing'])
def test_multilingual_audio_selection_is_deterministic(reverse, metadata):
    video = formats()[-1]
    audio = formats()[1]
    japanese = {**audio, 'format_id': '140-ja', 'language': 'ja', 'abr': 100}
    english = {**audio, 'format_id': '140-en', 'language': 'en', 'abr': 200}
    context = {}
    if metadata == 'preference':
        japanese['language_preference'], english['language_preference'] = 10, 5
    elif metadata == 'language':
        context['language'] = 'ja'
    elif metadata == 'default':
        japanese['format_note'] = 'Japanese (default)'
    items = [video, japanese, english]
    context['formats'] = items[::-1] if reverse else items
    selected = next(select_format(context, merge=True))
    assert [f['format_id'] for f in selected['requested_formats']] == ['137', '140-en' if metadata == 'missing' else '140-ja']
    key = selected['_yingqu_format_id']
    # Same itag and codec with changed language is a different actual download.
    changed = {**selected['requested_formats'][1], 'language': 'fr'}
    with pytest.raises(engine.EngineError, match='FORMAT_UNAVAILABLE'):
        next(select_format({'formats': [video, changed]}, key, merge=True))
    with pytest.raises(engine.EngineError, match='FORMAT_UNAVAILABLE'):
        next(select_format({'formats': [video]}, key, merge=True))


@pytest.mark.parametrize('extra', [{'ext': 'webm', 'acodec': 'opus'}, {'acodec': 'opus'},
    {'protocol': 'm3u8_native'}, {'has_drm': True}, {'format_note': 'watermarked'}])
def test_incompatible_audio_is_never_reported_as_complete(extra):
    with pytest.raises(engine.EngineError, match='UNSUPPORTED_CONTENT'):
        next(select_format({'formats': [formats()[-1], {**formats()[1], **extra}]}, merge=True))


def test_merged_identity_ignores_signatures_and_mirror_suffixes_and_preserves_mixed_size():
    items = formats()[1:]
    items[0]['filesize_approx'], items[0]['filesize'] = 30, None
    first = public_formats({'formats': items}, True)[0]
    assert first['filesize'] is None and first['filesize_approx'] == 130
    for item in items:
        item['url'] += '?signature=changed'
        item['format_id'] += '-2'
    second = public_formats({'formats': items}, True)[0]
    assert first['format_id'] == second['format_id']


def test_youtube_transport_remains_bounded():
    for domain in ('www.youtube.com', 'rr1.googlevideo.com', 'i.ytimg.com', 'youtubei.googleapis.com'):
        validate_target('https://' + domain + '/media', 'youtube')
    for domain in ('google.com', 'evilgooglevideo.com', 'googlevideo.com.evil.org', 'tiktokcdn.com'):
        with pytest.raises(TargetError):
            validate_target('https://' + domain + '/media', 'youtube')


@pytest.mark.parametrize('path', ['/sorry/index', '/sorry/'])
def test_youtube_challenge_is_rejected_as_risk_control(path):
    with pytest.raises(TargetError, match='^RISK_CONTROL$') as caught:
        validate_target('https://www.google.com' + path + '?continue=private', 'youtube')
    assert caught.value.code == 'RISK_CONTROL'


@pytest.mark.parametrize('url,platform', [
    ('https://www.google.com/sorry/index', 'tiktok'),
    ('https://www.google.com/search', 'youtube'),
    ('https://www.google.com.evil.org/sorry/index', 'youtube'),
    ('https://evilgoogle.com/sorry/index', 'youtube'),
    ('https://u:p@www.google.com/sorry/index', 'youtube'),
    ('https://www.google.com:444/sorry/index', 'youtube'),
    ('http://www.google.com/sorry/index', 'youtube'),
])
def test_other_rejected_targets_keep_their_classification(url, platform):
    with pytest.raises(TargetError, match='^NETWORK_TARGET_REJECTED$') as caught:
        validate_target(url, platform)
    assert caught.value.code == 'INVALID_INPUT'


def test_challenge_redirect_reports_risk_control_without_following(monkeypatch, media_proxy):
    proxy, requests, _, _ = media_proxy
    monkeypatch.setattr(engine, 'javascript_runtime', lambda: {'node': {'path': 'fixture-node'}})
    monkeypatch.setattr(engine, 'installed_tool', lambda _: None)
    def extract(self, url, download):
        self.urlopen('http://www.youtube.com/youtube_challenge')
        pytest.fail('challenge must stop extraction')
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    with pytest.raises(engine.EngineError, match='^RISK_CONTROL$'):
        engine.operate('resolve_video', {'input': URL, 'profiles': {'youtube': {'proxy': proxy}}},
                       lambda *_a, **_k: pytest.fail('challenge cannot produce a successful result'))
    assert len(requests) == 1
    assert requests[0][0] == 'http://www.youtube.com/youtube_challenge'


@pytest.mark.parametrize('route', ['wrapped', 'logged'])
def test_challenge_classification_survives_upstream_errors(monkeypatch, route):
    from yt_dlp.networking.exceptions import RequestError
    monkeypatch.setattr(engine, 'javascript_runtime', lambda: {'node': {'path': 'fixture-node'}})
    monkeypatch.setattr(engine, 'installed_tool', lambda _: None)
    def extract(self, url, download):
        try:
            validate_target('https://www.google.com/sorry/index', 'youtube')
        except TargetError as error:
            if route == 'wrapped':
                raise RequestError(cause=error)
            self.report_error(str(error))
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    with pytest.raises(engine.EngineError, match='^RISK_CONTROL$'):
        engine.operate('resolve_video', {'input': URL}, lambda *_a, **_k: None)


def test_default_browser_cookies_are_filtered_and_never_saved(monkeypatch):
    source, target = CookieJar(), CookieJar()
    engine.add_cookies(source, {'session': 'fixture-youtube'}, 'youtube')
    engine.add_cookies(source, {'session': 'fixture-tiktok'}, 'tiktok')
    calls = []
    monkeypatch.setattr(browser_cookies, 'default_browser', lambda: 'firefox')
    def extract(browser, **kwargs):
        calls.append(browser)
        return source
    monkeypatch.setattr(browser_cookies, 'extract_cookies_from_browser', extract)
    assert browser_cookies.load_default_cookies(target, 'youtube', engine.SafeLogger()) is None
    assert calls == ['firefox'] and [c.value for c in target] == ['fixture-youtube']


@pytest.mark.parametrize('browser', [None, 'chrome'])
def test_default_browser_failure_is_bounded_no_other_browser_retry(monkeypatch, browser):
    monkeypatch.setattr(browser_cookies, 'default_browser', lambda: browser)
    calls = []
    def extract(name, **kwargs):
        calls.append(name)
        raise OSError('private user profile path')
    monkeypatch.setattr(browser_cookies, 'extract_cookies_from_browser', extract)
    assert browser_cookies.load_default_cookies(CookieJar(), 'youtube', engine.SafeLogger()) == 'BROWSER_COOKIES_UNAVAILABLE'
    assert calls == ([] if browser is None else ['chrome'])


def test_upstream_cookie_warning_does_not_discard_other_cookies(monkeypatch):
    source = CookieJar()
    engine.add_cookies(source, {'session': 'fixture'}, 'youtube')
    monkeypatch.setattr(browser_cookies, 'default_browser', lambda: 'chrome')
    def extract(browser, logger):
        logger.warning('one cookie failed decryption', only_once=True)
        return source
    monkeypatch.setattr(browser_cookies, 'extract_cookies_from_browser', extract)
    target = CookieJar()
    assert browser_cookies.load_default_cookies(target, 'youtube', engine.SafeLogger()) is None
    assert [cookie.value for cookie in target] == ['fixture']


@pytest.mark.parametrize('cookie', ['', 'session=manual'])
def test_manual_cookie_priority_and_warning(monkeypatch, cookie):
    calls = []
    monkeypatch.setattr(engine, 'javascript_runtime', lambda: {'node': {'path': 'fixture-node'}})
    monkeypatch.setattr(engine, 'installed_tool', lambda _: None)
    monkeypatch.setattr(engine, 'load_default_cookies', lambda *_: calls.append('default') or 'BROWSER_COOKIES_UNAVAILABLE')
    def extract(self, url, download):
        return self.process_ie_result(dict(id='BaW_jenozKc', title='fixture', webpage_url=URL,
            extractor='fixture', extractor_key='Fixture', formats=formats()), download=False)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    events = []
    engine.operate('resolve_video', {'input': URL, 'profiles': {'youtube': {'cookie': cookie}}},
                   lambda kind, **data: events.append(data))
    assert calls == (['default'] if not cookie else [])
    assert bool(events[-1]['result'].get('cookie_warning')) == (not cookie)
    assert events[-1]['result']['formats'][0]['requires_merge']
    assert 'session' not in json.dumps(events)


def test_missing_ffmpeg_stops_before_media_io(monkeypatch, tmp_path):
    monkeypatch.setattr(engine, 'javascript_runtime', lambda: {'node': {'path': 'fixture-node'}})
    monkeypatch.setattr(engine, 'installed_tool', lambda _: None)
    def extract(self, url, download):
        return self.process_ie_result(dict(id='BaW_jenozKc', webpage_url=URL,
            extractor='fixture', extractor_key='Fixture', formats=formats()), download=False)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    monkeypatch.setattr(YoutubeDL, 'process_info', lambda *_: pytest.fail('media must not start'))
    with pytest.raises(engine.EngineError, match='FFMPEG_UNAVAILABLE'):
        engine.operate('download_video', {'input': URL, 'staging_dir': str(tmp_path),
            'expected': {'platform': 'youtube', 'video_id': 'BaW_jenozKc'}}, lambda *_a, **_k: None)
    assert list(tmp_path.iterdir()) == []


def test_missing_js_runtime_and_identification_without_runtime(monkeypatch):
    monkeypatch.setattr(engine, 'javascript_runtime', lambda: {})
    events = []
    engine.operate('identify_input', {'input': URL}, lambda kind, **data: events.append(data))
    assert events[0]['result']['video_id'] == 'BaW_jenozKc'
    with pytest.raises(engine.EngineError, match='JS_RUNTIME_UNAVAILABLE'):
        engine.operate('resolve_video', {'input': URL}, lambda *_a, **_k: None)


def test_runtime_version_detection(monkeypatch, tmp_path):
    suffix = '.exe' if sys.platform == 'win32' else ''
    for name in ('deno', 'node'):
        (tmp_path / (name + suffix)).write_bytes(b'fixture')
    monkeypatch.setattr(local_tools.shutil, 'which', lambda name: str(tmp_path / (name + suffix)))
    monkeypatch.setattr(local_tools, 'search_paths', lambda _: [])
    monkeypatch.setattr(local_tools, 'tool_output', lambda path, flag:
        'deno 2.2.0' if path.stem == 'deno' else 'v22.1.0')
    assert local_tools.javascript_runtime() == {'node': {'path': str(tmp_path / ('node' + suffix))}}


@pytest.mark.parametrize('old_output', ['', 'v20.0.0'])
def test_runtime_search_continues_after_old_or_unrunnable_path(monkeypatch, tmp_path, old_output):
    executable = 'node.exe' if sys.platform == 'win32' else 'node'
    old, current = tmp_path / 'old' / executable, tmp_path / 'manager' / executable
    for path in (old, current):
        path.parent.mkdir()
        path.write_bytes(b'fixture')
    monkeypatch.setattr(local_tools.shutil, 'which', lambda name: str(old) if name == 'node' else None)
    monkeypatch.setattr(local_tools, 'search_paths', lambda name: [current.parent] if name == 'node' else [])
    monkeypatch.setattr(local_tools, 'tool_output', lambda path, flag: old_output if path == old else 'v24.0.0')
    assert local_tools.javascript_runtime() == {'node': {'path': str(current)}}


def test_two_stream_download_merger_hooks_and_actual_output_size(monkeypatch, tmp_path, media_proxy):
    """Real HTTP downloads/yt-dlp orchestration, simulated local FFmpeg execution."""
    from conftest import BODY
    from yt_dlp.postprocessor.ffmpeg import FFmpegMergerPP
    proxy, requests, _, _ = media_proxy
    monkeypatch.setattr(engine, 'javascript_runtime', lambda: {'node': {'path': 'fixture-node'}})
    monkeypatch.setattr(engine, 'installed_tool', lambda _: None)
    # Exercise orchestration independently of installed FFmpeg; real execution is tested separately.
    original_tool = engine.installed_tool
    monkeypatch.setattr(engine, 'installed_tool', lambda name: 'fixture-ffmpeg' if name == 'ffmpeg' else original_tool(name))
    monkeypatch.setattr(FFmpegMergerPP, 'available', property(lambda _: True))
    merged = b'\x00\x00\x00\x18ftypmp42merged-fixture'
    def merge(self, inputs, output, args):
        assert len(inputs) == 2 and [Path(f).read_bytes() for f in inputs] == [BODY, BODY]
        assert args[:2] == ['-c', 'copy']
        Path(output).write_bytes(merged)
    monkeypatch.setattr(FFmpegMergerPP, 'run_ffmpeg_multiple_files', merge)
    def extract(self, url, download):
        items = formats()[1:]
        for item in items:
            item.pop('filesize', None)
            item['url'] = 'http://v.fixture.googlevideo.com/good.mp4'
            item['protocol'] = 'http'
        return self.process_ie_result(dict(id='BaW_jenozKc', webpage_url=URL, title='fixture',
            extractor='fixture', extractor_key='Fixture', formats=items), download=False)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    events = []
    engine.operate('download_video', {'input': URL, 'staging_dir': str(tmp_path),
        'profiles': {'youtube': {'proxy': proxy}},
        'expected': {'platform': 'youtube', 'video_id': 'BaW_jenozKc'}},
        lambda kind, **data: events.append((kind, data)))
    progress = [data for kind, data in events if kind == 'progress']
    counts = [data['downloaded_bytes'] for data in progress]
    assert counts == sorted(counts) and counts[-1] == 2 * len(BODY)
    assert progress[-1]['phase'] == 'merging'
    terminal = events[-1][1]
    assert terminal['bytes'] == len(merged) and terminal['transferred_bytes'] == 2 * len(BODY)
    assert (tmp_path / 'video.mp4').read_bytes() == merged
    assert list(tmp_path.iterdir()) == [tmp_path / 'video.mp4']
    assert len(requests) == 2


def test_real_installed_ffmpeg_merges_both_streams(monkeypatch, tmp_path, media_proxy, media_bodies):
    import subprocess
    ffmpeg = local_tools.installed_tool('ffmpeg')
    if not ffmpeg:
        pytest.skip('No runnable system FFmpeg installed for real merge validation')
    video_file, audio_file = tmp_path / 'source.mp4', tmp_path / 'source.m4a'
    flags = {'capture_output': True, 'timeout': 15,
             'creationflags': 0x08000000 if sys.platform == 'win32' else 0}
    subprocess.run([ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'color=c=red:s=320x180:r=5',
        '-t', '0.4', '-an', '-c:v', 'mpeg4', str(video_file)], check=True, **flags)
    subprocess.run([ffmpeg, '-v', 'error', '-f', 'lavfi', '-i', 'anullsrc=r=44100:cl=stereo',
        '-t', '0.4', '-vn', '-c:a', 'aac', str(audio_file)], check=True, **flags)
    media_bodies.update(ytvideo=video_file.read_bytes(), ytaudio=audio_file.read_bytes())
    proxy, _, _, _ = media_proxy
    stage = tmp_path / 'stage'
    stage.mkdir()
    def extract(self, url, download):
        items = formats()[1:]
        for item, name in zip(items, ('ytaudio', 'ytvideo')):
            item['filesize'] = len(media_bodies[name])
            item['url'] = f'http://v.fixture.googlevideo.com/{name}.mp4'
            item['protocol'] = 'http'
        return self.process_ie_result(dict(id='BaW_jenozKc', webpage_url=URL, title='fixture',
            extractor='fixture', extractor_key='Fixture', formats=items), download=False)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    events = []
    engine.operate('download_video', {'input': URL, 'staging_dir': str(stage),
        'profiles': {'youtube': {'proxy': proxy}},
        'expected': {'platform': 'youtube', 'video_id': 'BaW_jenozKc'}},
        lambda kind, **data: events.append((kind, data)))
    terminal = events[-1][1]
    assert terminal['bytes'] == (stage / 'video.mp4').stat().st_size
    assert terminal['transferred_bytes'] == sum(map(len, media_bodies.values()))
    assert any(data.get('phase') == 'merging' for kind, data in events)
    probe = Path(ffmpeg).with_name('ffprobe.exe' if sys.platform == 'win32' else 'ffprobe')
    if probe.is_file():
        result = subprocess.run([str(probe), '-v', 'error', '-show_entries', 'stream=codec_name,codec_type',
                                 '-of', 'json', str(stage / 'video.mp4')], check=True, **flags)
        streams = json.loads(result.stdout)['streams']
        assert {(s['codec_type'], s['codec_name']) for s in streams} == {('video', 'mpeg4'), ('audio', 'aac')}


def test_tool_finder_skips_bad_path_and_checks_install_locations(monkeypatch, tmp_path):
    executable = 'ffmpeg.exe' if sys.platform == 'win32' else 'ffmpeg'
    stale, installed = tmp_path / 'stale' / executable, tmp_path / 'manager' / executable
    for path in (stale, installed):
        path.parent.mkdir()
        path.write_bytes(b'fixture')
    monkeypatch.setattr(local_tools.shutil, 'which', lambda _: str(stale))
    monkeypatch.setattr(local_tools, 'search_paths', lambda _: [stale.parent, installed.parent])
    monkeypatch.setattr(local_tools, 'tool_output', lambda path, _: '' if path == stale else 'ffmpeg version 8.0')
    assert local_tools.installed_tool('ffmpeg') == str(installed)


def test_macos_gui_search_includes_both_homebrew_and_macports(monkeypatch, tmp_path):
    monkeypatch.setattr(local_tools.sys, 'platform', 'darwin')
    monkeypatch.setattr(local_tools.Path, 'home', lambda: tmp_path)
    monkeypatch.setenv('HOMEBREW_PREFIX', str(tmp_path / 'brew'))
    roots = local_tools.search_paths('ffmpeg')
    for path in ('/opt/homebrew/bin', '/usr/local/bin', '/opt/local/bin',
                 '/opt/homebrew/opt/ffmpeg/bin', '/usr/local/opt/ffmpeg/bin',
                 '/opt/homebrew/opt/ffmpeg-full/bin', '/usr/local/opt/ffmpeg-full/bin'):
        assert Path(path) in roots
    assert tmp_path / 'brew/bin' in roots


def test_windows_registry_path_refreshed_when_process_path_is_stale(monkeypatch, tmp_path):
    from types import SimpleNamespace
    class Key:
        def __init__(self, hive):
            self.hive = hive
        def __enter__(self):
            return self
        def __exit__(self, *_):
            pass
    locations = {'user': str(tmp_path / 'user-bin'), 'machine': str(tmp_path / 'machine-bin')}
    fake = SimpleNamespace(HKEY_CURRENT_USER='user', HKEY_LOCAL_MACHINE='machine',
        OpenKey=lambda hive, key: Key(hive), QueryValueEx=lambda key, name: (locations[key.hive], 1))
    monkeypatch.setitem(sys.modules, 'winreg', fake)
    assert local_tools.windows_path() == [Path(locations['user']), Path(locations['machine'])]


def test_winget_portable_package_found_without_links_or_path(monkeypatch, tmp_path):
    monkeypatch.setattr(local_tools.sys, 'platform', 'win32')
    monkeypatch.setattr(local_tools.Path, 'home', lambda: tmp_path)
    monkeypatch.setattr(local_tools, 'windows_path', lambda: [])
    monkeypatch.setenv('LOCALAPPDATA', str(tmp_path / 'Local'))
    executable = tmp_path / 'Local/Microsoft/WinGet/Packages/Gyan.FFmpeg_source/ffmpeg-8/bin/ffmpeg.exe'
    executable.parent.mkdir(parents=True)
    executable.write_bytes(b'fixture')
    assert executable.parent in local_tools.search_paths('ffmpeg')
