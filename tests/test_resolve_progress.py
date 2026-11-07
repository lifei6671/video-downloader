"""Actual engine phase boundaries with extraction/preview I/O replaced by offline fixtures."""
from contextlib import nullcontext
from types import SimpleNamespace

import pytest
from yt_dlp import YoutubeDL

import engine

URL = 'https://www.tiktok.com/@fixture/video/123'


def fixture_engine(monkeypatch, events, enabled):
    def current(phase):
        if enabled:
            assert events[-1] == ('resolve_progress', {'phase': phase})

    def cookies(*args):
        current('loading_configuration')

    def extract(self, url, download):
        current('fetching_metadata')
        return self.process_ie_result(dict(id='123', webpage_url=url, title='fixture',
            extractor='fixture', extractor_key='Fixture', formats=[dict(format_id='v',
                ext='mp4', protocol='http', vcodec='h264', acodec='aac', width=640, height=360,
                url='https://v.tiktokcdn.com/fixture.mp4')]), download=False)

    original_metadata = engine.metadata

    def metadata(*args):
        current('selecting_formats')
        return original_metadata(*args)

    def cover(*args):
        current('loading_preview')
        return {}

    def avatar(*args):
        current('loading_author')
        return None

    monkeypatch.setattr(engine, 'load_default_cookies', cookies)
    monkeypatch.setattr(YoutubeDL, 'extract_info', extract)
    monkeypatch.setattr(engine, 'metadata', metadata)
    monkeypatch.setattr(engine, 'download_cover', cover)
    monkeypatch.setattr(engine.author_profile, 'avatar_url', avatar)
    monkeypatch.setattr(engine.author_profile, 'download_avatar', lambda *args: {})


@pytest.mark.parametrize('enabled', [True, False])
def test_resolve_phases_precede_real_operations_and_legacy_calls_remain_quiet(monkeypatch, tmp_path, enabled):
    events = []
    fixture_engine(monkeypatch, events, enabled)
    engine.operate('resolve_video', dict(input=URL, profiles={}, preview_dir=str(tmp_path),
        report_resolve_progress=enabled), lambda kind, **data: events.append((kind, data)))
    phases = [data['phase'] for kind, data in events if kind == 'resolve_progress']
    assert phases == (['loading_configuration', 'fetching_metadata', 'selecting_formats',
                       'loading_preview', 'loading_author'] if enabled else [])
    assert events[-1][0] == 'resolved'
    assert all(set(data) == {'phase'} for kind, data in events if kind == 'resolve_progress')


def test_short_link_identification_reports_only_executed_steps(monkeypatch):
    events = []
    fixture_engine(monkeypatch, events, True)

    def open_link(self, url):
        assert events[-1] == ('resolve_progress', {'phase': 'resolving_link'})
        return nullcontext(SimpleNamespace(url=URL))

    monkeypatch.setattr(YoutubeDL, 'urlopen', open_link)
    engine.operate('identify_input', dict(input='https://vm.tiktok.com/fixture/', profiles={},
        report_resolve_progress=True), lambda kind, **data: events.append((kind, data)))
    assert [data['phase'] for kind, data in events if kind == 'resolve_progress'] == ['loading_configuration', 'resolving_link']
    assert events[-1][0] == 'identified'


def test_extraction_failure_does_not_claim_later_steps(monkeypatch, tmp_path):
    events = []
    fixture_engine(monkeypatch, events, True)

    def fail(*args, **kwargs):
        raise engine.EngineError('ACCESS_DENIED', 'fixture')

    monkeypatch.setattr(YoutubeDL, 'extract_info', fail)
    with pytest.raises(engine.EngineError, match='ACCESS_DENIED'):
        engine.operate('resolve_video', dict(input=URL, profiles={}, preview_dir=str(tmp_path),
            report_resolve_progress=True), lambda kind, **data: events.append((kind, data)))
    assert [data['phase'] for kind, data in events] == ['loading_configuration', 'fetching_metadata']


@pytest.mark.parametrize('value', [None, 1, 'true', {}])
def test_progress_opt_in_is_boolean(value):
    with pytest.raises(engine.EngineError, match='CONFIG_INVALID'):
        engine.operate('resolve_video', dict(input=URL, report_resolve_progress=value), lambda *args, **kw: None)
