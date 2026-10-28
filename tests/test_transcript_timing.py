"""Regression checks for the WB-301 experiment's timing evidence, not ASR mocks."""
import importlib.util
from pathlib import Path

import pytest

spec = importlib.util.spec_from_file_location('verify_transcript_timing', Path(__file__).resolve().parents[1] / 'scripts/verify_transcript_timing.py')
timing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(timing)


def test_vad_retains_silence_gaps_and_clamps_native_eof():
    assert timing.intervals('100 800\n1800 3020\n', 3001) == [(100, 800), (1800, 3001)]


@pytest.mark.parametrize('text', ['-1 100', '0 0', '100 50', '0 3032', '3001 3010', '0.5 100',
                                  '0 100\n90 200', 'progress 1 2', '0 100 extra'])
def test_bad_native_ranges_cannot_become_timing_evidence(text):
    with pytest.raises(ValueError):
        timing.intervals(text, 3001)


def test_srt_unicode_multiline_and_real_window_mapping():
    text = '1\n00:00:00,100 --> 00:00:00,800\n中文😀\nEnglish\n\n2\n00:01:01,000 --> 00:01:02,500\n结尾\n'
    cues = timing.parse_srt(text)
    assert cues[0]['text'] == '中文😀\nEnglish'
    assert cues[1]['start_ms'] == 61000
    assert timing.match_cues(cues, [(100, 800), (1000, 1200), (61000, 62500)]) == [0, 2]
    assert timing.parse_srt('') == []


@pytest.mark.parametrize('text', ['1\n00:00:00,100 --> 00:00:00,100\n字',
                                  '1\n00:00:60,000 --> 00:01:01,000\n字',
                                  '2\n00:00:00,000 --> 00:00:01,000\n字',
                                  '1\n00:00:00,000 --> 00:00:01,000',
                                  '1\n00:00:00,100 --> 00:00:00,800\n字\n\n2\n00:00:00,700 --> 00:00:00,900\n字'])
def test_srt_bad_timing_cannot_pass(text):
    with pytest.raises(ValueError):
        timing.parse_srt(text)


def test_text_length_must_not_manufacture_or_reorder_ranges():
    for cues in [[dict(start_ms=0, end_ms=500, text='字')],
                 [dict(start_ms=1000, end_ms=2000, text='A'), dict(start_ms=0, end_ms=1000, text='B')]]:
        with pytest.raises(ValueError):
            timing.match_cues(cues, [(0, 1000), (1000, 2000)])


def test_controlled_audio_keeps_known_silence_and_original_inputs(tmp_path, monkeypatch):
    monkeypatch.setattr(timing, 'ROOT', tmp_path)
    assets = tmp_path / 'python-engine/assets'
    assets.mkdir(parents=True)
    zh, en, natural = assets / 'sensevoice-smoke.wav', tmp_path / 'en.wav', tmp_path / 'natural.wav'
    for path, frames in [(zh, 48000), (en, 32000), (natural, 96001)]:
        timing.write_pcm(path, b'\x01\x01' * frames)
    before = [timing.digest(p) for p in [zh, en, natural]]
    output = tmp_path / 'output'
    output.mkdir()
    cases = timing.prepare_cases(output, en, natural)
    assert [c['name'] for c in cases] == ['short-zh', 'pause-zh-en', 'continuous-zh', 'natural-zh']
    mixed = cases[1]
    assert mixed['placement'] == [dict(language='zh', start_ms=800, end_ms=3800),
                                  dict(language='en', start_ms=5600, end_ms=7600),
                                  dict(language='zh', start_ms=9400, end_ms=12400)]
    pcm = timing.read_pcm(mixed['path'])
    assert pcm[:800 * 32] == bytes(800 * 32)
    assert pcm[3800 * 32:5600 * 32] == bytes(1800 * 32)
    assert len(timing.read_pcm(cases[2]['path'])) == 22 * 48000 * 2
    assert len(timing.read_pcm(cases[3]['path'])) == 96001 * 2
    assert [timing.digest(p) for p in [zh, en, natural]] == before
