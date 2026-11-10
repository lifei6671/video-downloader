from types import SimpleNamespace

import pytest

import engine
import douyin_author


AUTHOR = 'fixture-author'
URL = f'https://www.douyin.com/user/{AUTHOR}'


def test_identify_direct_profile_without_expanding(monkeypatch):
    monkeypatch.setattr(douyin_author, 'expand_share', lambda *_args: pytest.fail('direct link expanded'))
    events = []
    douyin_author.identify_link({'input': f'查看TA的更多作品。 {URL}'},
                               lambda kind, **value: events.append((kind, value)))
    assert events == [('identified', {'result': {'kind': 'author', 'author_id': AUTHOR}})]


@pytest.mark.parametrize(('target', 'result'), [
    (f'https://www.iesdouyin.com/share/user/{AUTHOR}', {'kind': 'author', 'author_id': AUTHOR}),
    ('https://www.douyin.com/video/123', {'kind': 'work', 'canonical_url': 'https://www.douyin.com/video/123'}),
    ('https://www.douyin.com/note/456', {'kind': 'work', 'canonical_url': 'https://www.douyin.com/note/456'}),
])
def test_identify_short_link_dispatches_after_one_expansion(monkeypatch, target, result):
    short = 'https://v.douyin.com/fixture/'
    expanded = []

    def expand(_client, url, _ua):
        expanded.append(url)
        return target

    monkeypatch.setattr(douyin_author, 'expand_share', expand)
    events = []
    douyin_author.identify_link({'input': short}, lambda kind, **value: events.append((kind, value)))
    assert expanded == [short]
    assert events == [('identified', {'result': result})]


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
    waits = []
    offsets = iter([500, 1200] * 21)

    def jitter(min_ms, max_ms):
        assert (min_ms, max_ms) == (500, 1200)
        return next(offsets)

    def page(*_args, cursor, **_kwargs):
        cursor_seen.append(cursor)
        number = len(cursor_seen)
        return dict(status_code=0, aweme_list=[], has_more=int(number < 42), max_cursor=number)

    monkeypatch.setattr(douyin_author.dtk_adapter, 'request_json', page)
    monkeypatch.setattr(douyin_author.time, 'monotonic', lambda: 0)
    monkeypatch.setattr(douyin_author.time, 'sleep', waits.append)
    monkeypatch.setattr(douyin_author.random, 'randint', jitter)
    events = []
    douyin_author.scan({'input': URL, 'profile': {'cookie': 'sessionid=fixture'}},
                       lambda event, **value: events.append((event, value)))
    assert cursor_seen == [None, *map(str, range(1, 42))]
    assert waits == pytest.approx([9.0, 9.7] * 20 + [9.0])
    assert events[-1] == ('author_scan_finished', {'result': {'pages': 42}})


def test_share_query_does_not_override_author_and_posts_are_not_profiles():
    assert douyin_author.profile_link(URL + '?sec_uid=another-author')[1] == AUTHOR
    with pytest.raises(engine.EngineError, match='INVALID_AUTHOR_LINK'):
        douyin_author.profile_link('https://www.douyin.com/video/123')
