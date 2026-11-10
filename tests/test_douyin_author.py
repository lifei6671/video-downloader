from types import SimpleNamespace

import pytest

import engine
import douyin_author


AUTHOR = 'fixture-author'
URL = f'https://www.douyin.com/user/{AUTHOR}'


@pytest.mark.parametrize('value', [None, 2, 'unknown', 'false', [], 0.0])
def test_invalid_has_more_cannot_end_scan(value):
    with pytest.raises(engine.EngineError, match='UPSTREAM_CHANGED'):
        douyin_author.parse_page({'status_code': 0, 'aweme_list': [], 'has_more': value}, AUTHOR)


def test_mixed_authors_keep_only_the_requested_authors_posts(monkeypatch):
    def post(id, author, kind):
        return SimpleNamespace(content_id=id, author=SimpleNamespace(sec_uid=author, nickname='Fixture'),
                               kind=kind, is_private=False, is_deleted=False, title='title', created_at=None)

    items = [post('1', AUTHOR, 'video'), post('2', 'different-author', 'video'),
             post('3', AUTHOR, 'image_album')]
    monkeypatch.setattr(douyin_author.dtk_adapter, 'DOUYIN', SimpleNamespace(
        parse_author_posts=lambda *_args, **_kwargs: SimpleNamespace(items=items, has_more=False, cursor=None)))
    _, parsed, skipped = douyin_author.parse_page({'status_code': 0, 'aweme_list': [], 'has_more': 0}, AUTHOR)
    assert [(item['video_id'], item['kind']) for item in parsed] == [('1', 'video'), ('3', 'image_album')]
    assert skipped == 1


def test_foreign_post_with_missing_media_does_not_break_page():
    _, parsed, skipped = douyin_author.parse_page(
        {'status_code': 0, 'aweme_list': [{'author': {'sec_uid': 'different-author'}}], 'has_more': 0},
        AUTHOR)
    assert not parsed
    assert skipped == 1


def test_scan_reads_past_forty_pages_and_validates_the_end(monkeypatch):
    cursor_seen = []

    def page(*_args, cursor, **_kwargs):
        cursor_seen.append(cursor)
        number = len(cursor_seen)
        return dict(status_code=0, aweme_list=[], has_more=int(number < 42), max_cursor=number)

    monkeypatch.setattr(douyin_author.dtk_adapter, 'request_json', page)
    monkeypatch.setattr(douyin_author.time, 'sleep', lambda _seconds: None)
    events = []
    douyin_author.scan({'input': URL, 'profile': {'cookie': 'sessionid=fixture'}},
                       lambda event, **value: events.append((event, value)))
    assert cursor_seen == [None, *map(str, range(1, 42))]
    assert events[-1] == ('author_scan_finished', {'result': {'pages': 42}})


def test_share_query_does_not_override_author_and_posts_are_not_profiles():
    assert douyin_author.profile_link(URL + '?sec_uid=another-author')[1] == AUTHOR
    with pytest.raises(engine.EngineError, match='INVALID_AUTHOR_LINK'):
        douyin_author.profile_link('https://www.douyin.com/video/123')
