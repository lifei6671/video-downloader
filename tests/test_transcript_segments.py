import json

import pytest

import transcript_segments as segments


def test_cpu_srt_keeps_actual_windows_unicode_and_gaps(tmp_path):
    text = '1\n00:00:00,120 --> 00:00:00,400\n中文😀\nEnglish\n\n2\n00:00:00,600 --> 00:00:01,000\n下一段。\n'
    rows = segments.parse_srt(text, 16001)
    descriptor = segments.write(tmp_path, rows, 16001, 'vad-revision')
    artifact = json.loads((tmp_path / descriptor['filename']).read_text('utf-8'))
    assert artifact['frames'] == 16001 and artifact['precision'] == 'vad_segment'
    assert artifact['segments'] == [dict(start_ms=120, end_ms=400, text='中文😀\nEnglish'), dict(start_ms=600, end_ms=1000, text='下一段。')]
    assert descriptor['bytes'] == (tmp_path / descriptor['filename']).stat().st_size
    assert segments.parse_srt('', 16000) == []


@pytest.mark.parametrize('cue', [
    '1\n00:00:00,000 --> 00:00:01,001\n超界',
    '1\n00:00:00,400 --> 00:00:00,400\n空范围',
    '2\n00:00:00,000 --> 00:00:00,100\n错序',
    '1\n00:00:60,000 --> 00:01:01,000\n错钟',
    '1\n00:00:00,000 --> 00:00:00,100\n/sil',
    '1\n00:00:00,000 --> 00:00:00,800\n第一\n\n2\n00:00:00,700 --> 00:00:01,000\n重叠',
    '原始全文不能假装有时间',
])
def test_bad_srt_is_rejected_without_interpolated_times(cue):
    with pytest.raises(ValueError):
        segments.parse_srt(cue, 16000)


def test_bad_artifact_does_not_create_success_file(tmp_path):
    with pytest.raises(ValueError):
        segments.write(tmp_path, [dict(start_ms=True, end_ms=1000, text='字')], 16000, 'vad')
    assert not (tmp_path / segments.FILENAME).exists()
