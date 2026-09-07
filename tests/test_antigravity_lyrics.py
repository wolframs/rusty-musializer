"""Offline controls for audio consent, clip seams, ACP transport and caching."""
import argparse
import asyncio
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import antigravity_audio as acp
import antigravity_lyrics as lyrics
import external_analysis


def line(text='A performed line', start=1.0, end=3.0, **extra):
    return dict(text=text, start_seconds=start, end_seconds=end,
                uncertain=False, partial_start=False, partial_end=False, **extra)


def response(rows):
    return json.dumps(dict(lines=rows, notes=[]))


class ParsingAndSeams(unittest.TestCase):
    def test_invalid_remote_payloads_are_not_captions(self):
        row = line()
        for changed in ({'start_seconds': float('nan')}, {'end_seconds': 31},
                        {'start_seconds': 30.01, 'end_seconds': 30.02},
                        {'start_seconds': True}, {'partial_start': 'false'},
                        {'text': '\tname'}, {'text': 'x' * 513}):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                lyrics.parse_response(response([{**row, **changed}]), 30)
        self.assertEqual(lyrics.parse_response(response([]), 30), [])

    def test_overlapping_clips_cover_the_full_track_without_tiny_last_request(self):
        self.assertEqual(lyrics.clip_spans(46), [(0, 30), (15, 30), (30, 16)])
        self.assertEqual(lyrics.clip_spans(30), [(0, 30)])
        self.assertEqual(lyrics.clip_spans(15.1), [(0, 15.1)])

    def test_duplicate_cross_clip_observation_does_not_delete_a_repeat(self):
        reports = [dict(clip_start_seconds=0, clip_duration_seconds=30,
                        lines=[line('Again', 17, 19), line('Again', 20, 22)]),
                   dict(clip_start_seconds=15, clip_duration_seconds=30,
                        lines=[line('Again', 2.1, 4.1), line('Again', 5.1, 7.1)])]
        merged, unresolved = lyrics.merge_observations(reports, 45)
        self.assertEqual(len(merged), 2)
        self.assertEqual(unresolved, [])
        self.assertEqual([r['start_seconds'] for r in merged], [17.05, 20.05])
        self.assertEqual([r['evidence_indices'] for r in merged], [[0, 1], [0, 1]])

    def test_transitive_merge_cannot_erase_two_deliveries_from_one_observation(self):
        reports = [dict(clip_start_seconds=0, clip_duration_seconds=30,
                        lines=[line('Again', 17, 20)]),
                   dict(clip_start_seconds=15, clip_duration_seconds=30,
                        lines=[line('Again', 2.1, 5.1), line('Again', 2.6, 5.6)])]
        merged, _ = lyrics.merge_observations(reports, 45)
        self.assertEqual(len(merged), 2)

    def test_written_dropped_g_can_corroborate_a_nearby_complete_phrase(self):
        reports = [dict(clip_start_seconds=0, clip_duration_seconds=30,
                        lines=[line('Cooking in Texas', 17, 19)]),
                   dict(clip_start_seconds=15, clip_duration_seconds=30,
                        lines=[line('Cookin’ in Texas', 2.3, 4.4)])]
        merged, unresolved = lyrics.merge_observations(reports, 45)
        self.assertEqual(len(merged), 1)
        self.assertTrue(lyrics.corroborated(merged[0]))
        self.assertEqual(unresolved, [])
        self.assertTrue(merged[0]['observation_disagreement'])
        self.assertTrue(merged[0]['uncertain'])
        self.assertEqual(merged[0]['text'], 'Cooking in Texas')
        self.assertAlmostEqual(merged[0]['start_seconds'], 17.15)
        self.assertAlmostEqual(merged[0]['end_seconds'], 19.2)

    def test_elision_matching_does_not_merge_different_words_or_deliveries(self):
        for text, start, end in [('Looking in Texas', 2.3, 4.4),
                                 ('Cookin in Texas', 2.3, 4.4),
                                 ("Cookin' in Texas", 4, 6),
                                 ("Cookin' in Texas", 2.3, 7)]:
            with self.subTest(text=text, start=start, end=end):
                rows, _ = lyrics.merge_observations([
                    dict(clip_start_seconds=0, clip_duration_seconds=30,
                         lines=[line('Cooking in Texas', 17, 19)]),
                    dict(clip_start_seconds=15, clip_duration_seconds=30,
                         lines=[line(text, start, end)])], 45)
                self.assertEqual(len(rows), 2)
                self.assertTrue(all(not lyrics.corroborated(row) for row in rows))
        # A single crop's repeated deliveries never corroborate one another.
        rows, _ = lyrics.merge_observations([dict(clip_start_seconds=0,
            clip_duration_seconds=30, lines=[line('Cooking in Texas', 17, 19),
                                            line("Cookin' in Texas", 17.3, 19.4)])], 30)
        self.assertEqual(len(rows), 2)
        self.assertTrue(all(not lyrics.corroborated(row) for row in rows))

    def test_fusion_resists_a_wrong_ctc_occurrence(self):
        row = dict(start_seconds=13.0, end_seconds=16.0,
                   timing_observations=[dict(start_seconds=12.1, end_seconds=14.9, complete=True),
                                        dict(start_seconds=12.0, end_seconds=15.0, complete=True)])
        self.assertEqual(lyrics.fused_boundaries(row), (12.1, 15.0))
        row['timing_observations'].pop()
        self.assertEqual(lyrics.fused_boundaries(row), (12.1, 14.9))

    def test_acoustic_search_contains_both_complete_observations(self):
        row = dict(start_seconds=117.6, end_seconds=118.45,
                   timing_observations=[dict(start_seconds=115.4, end_seconds=116.08, complete=True)])
        self.assertEqual(lyrics.observation_window(row, 120), (114.65, 119.2))

    def test_complete_observation_replaces_its_clipped_copy(self):
        partial = line('Again', 28, 30); partial['partial_end'] = True
        reports = [dict(clip_start_seconds=0, clip_duration_seconds=30, lines=[partial]),
                   dict(clip_start_seconds=15, clip_duration_seconds=30, lines=[line('Again', 13, 17)])]
        merged, unresolved = lyrics.merge_observations(reports, 45)
        self.assertEqual(len(merged), 1)
        self.assertEqual(merged[0]['end_seconds'], 32)
        self.assertEqual(unresolved, [])

    def test_competing_versions_are_parked_but_jointly_heard_backing_is_preserved(self):
        lead = dict(line('Go go go', 12, 15), source_line_indices=[0], edge_margin=12)
        alternative = dict(line('No no no', 12.1, 15.2), source_line_indices=[1], edge_margin=2)
        backing = dict(line('I really think so', 12.4, 14), source_line_indices=[0], edge_margin=10)
        kept, parked = lyrics.competing_observations([alternative, lead, backing])
        self.assertEqual([r['text'] for r in kept], ['Go go go', 'I really think so'])
        self.assertEqual([r['text'] for r in parked], ['No no no'])
        self.assertTrue(kept[0]['uncertain'])

    def test_full_phrase_and_its_fragment_are_not_both_rendered(self):
        full = dict(line('Started like a groyper but now I am an idol', 13.8, 19.8),
                    source_line_indices=[0, 1], edge_margin=10)
        fragment = dict(line('but now I am an idol', 17.2, 19.6),
                        source_line_indices=[1], edge_margin=2)
        repeat = dict(line('but now I am an idol', 21.2, 23.6),
                      source_line_indices=[1], edge_margin=6)
        kept, parked = lyrics.competing_observations([full, fragment, repeat])
        self.assertEqual([r['start_seconds'] for r in kept], [13.8, 21.2])
        self.assertEqual([r['start_seconds'] for r in parked], [17.2])

    def test_uncovered_partial_is_recorded_not_called_a_complete_line(self):
        partial = line('I began', 28, 30); partial['partial_end'] = True
        merged, unresolved = lyrics.merge_observations([
            dict(clip_start_seconds=0, clip_duration_seconds=30, lines=[partial])], 45)
        self.assertEqual(merged, [])
        self.assertEqual(len(unresolved), 1)

    def test_later_central_crop_cannot_erase_an_earlier_disagreement(self):
        # All three crops see the same delivery. The third wins caption rank,
        # but agrees only with the second: the first disagreement still exists.
        reports = [dict(clip_start_seconds=0, clip_duration_seconds=15,
                        lines=[line('We follow every signal', 11, 13)]),
                   dict(clip_start_seconds=2, clip_duration_seconds=20,
                        lines=[line('We follow any signal', 9, 11)]),
                   dict(clip_start_seconds=1, clip_duration_seconds=25,
                        lines=[line('We follow any signal', 10, 12)])]
        before, _ = lyrics.merge_observations(reports[:2], 30)
        self.assertTrue(before[0]['observation_disagreement'])
        merged, _ = lyrics.merge_observations(reports, 30)
        self.assertEqual(len(merged), 1)
        self.assertTrue(merged[0].get('observation_disagreement'))
        self.assertTrue(merged[0]['uncertain'])
        self.assertEqual(merged[0]['evidence_indices'], [0, 1, 2])
        self.assertEqual((merged[0]['start_seconds'], merged[0]['end_seconds']), (11, 13))

    def test_disagreeing_observations_remain_uncertain(self):
        merged, _ = lyrics.merge_observations([
            dict(clip_start_seconds=0, clip_duration_seconds=30, lines=[line(start=17, end=20)]),
            dict(clip_start_seconds=15, clip_duration_seconds=30, lines=[line(start=2.6, end=5.6)])], 45)
        self.assertEqual(len(merged), 1)
        self.assertTrue(merged[0]['uncertain'])

    def test_clock_drift_does_not_duplicate_a_whole_fast_passage(self):
        reports = [dict(clip_start_seconds=0, clip_duration_seconds=30,
                        lines=[line('First phrase', 17, 17.7), line('Second phrase', 18, 18.7),
                               line('Again', 20, 20.7), line('Again', 21, 21.7)]),
                   dict(clip_start_seconds=15, clip_duration_seconds=30,
                        lines=[line('First phrase', 3.2, 3.9), line('Second phrase', 4.2, 4.9),
                               line('Again', 6.2, 6.9), line('Again', 7.2, 7.9)])]
        merged, unresolved = lyrics.merge_observations(reports, 45)
        self.assertEqual(len(merged), 4)
        self.assertEqual(unresolved, [])
        self.assertEqual([r['evidence_indices'] for r in merged], [[0, 1]] * 4)
        self.assertTrue(all(r['uncertain'] for r in merged))
        self.assertEqual([len(r['timing_observations']) for r in merged], [2] * 4)


class ConsentAndCache(unittest.IsolatedAsyncioTestCase):
    async def test_authored_sheet_keeps_discovery_as_coarse_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            audio = root / 'audio'; audio.write_bytes(b'audio')
            server = root / 'server'; server.write_bytes(b'runtime')
            harness = root / 'harness'; harness.write_bytes(b'harness')
            reference = dict(text='Private authored wording', source='explicit', sha256='reference')
            answer = dict(agent={'name': 'antigravity-acp'}, model=acp.MODELS[0],
                          session_id='test', response=response([line('Different heard wording')]))
            with patch.object(acp, 'discover', return_value=dict(server=server, harness=harness, profile=root)), \
                 patch.object(acp, 'ask', new=AsyncMock(return_value=answer)) as ask, \
                 patch.object(lyrics.subprocess, 'check_output', return_value=b'wav'):
                value = await lyrics.transcribe(audio, root / 'out.json', length=12,
                    model=acp.MODELS[0], confirmed=True, reference=reference)
            self.assertEqual(value['schema_version'], 'musializer.lyric-timing/v1')
            self.assertEqual(value['source']['text_authority'], 'coarse-audio-evidence')
            self.assertEqual(value['source']['reference'], reference)
            self.assertEqual(value['source']['boundary_observations'][0]['lines'],
                             [line('Different heard wording')])
            self.assertEqual(value['lines'][0]['text'], 'Different heard wording')
            self.assertEqual(ask.await_count, 1)
            self.assertNotIn(reference['text'], ask.call_args.args[2])

    async def test_retry_names_invalid_boundary_and_exact_clip_duration(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            audio = root / 'audio'; audio.write_bytes(b'audio')
            server = root / 'server'; server.write_bytes(b'runtime')
            harness = root / 'harness'; harness.write_bytes(b'harness')
            metadata = dict(agent={'name': 'antigravity-acp'}, model=acp.MODELS[0], session_id='test')
            answers = [{**metadata, 'response': response([line(end=17.4)])},
                       {**metadata, 'response': response([])}]
            with patch.object(acp, 'discover', return_value=dict(server=server, harness=harness, profile=root)), \
                 patch.object(acp, 'ask', new=AsyncMock(side_effect=answers)) as ask, \
                 patch.object(lyrics.subprocess, 'check_output', return_value=b'wav'):
                value = await lyrics.transcribe(audio, root / 'out.json', length=17.08,
                    model=acp.MODELS[0], confirmed=True)
            self.assertEqual(value['lines'], [])
            retry = ask.call_args.args[2]
            self.assertIn('17.080000 seconds', retry)
            self.assertIn('invalid clip-relative timing', retry)
            self.assertNotIn('A performed line', retry)
            self.assertEqual(ask.await_count, 2)

    async def test_unconfirmed_hallucination_never_becomes_an_active_caption(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            audio = root / 'audio'; audio.write_bytes(b'audio')
            server = root / 'server'; server.write_bytes(b'runtime')
            harness = root / 'harness'; harness.write_bytes(b'harness')
            metadata = dict(agent={'name': 'antigravity-acp'}, model=acp.MODELS[0], session_id='test')
            answers = [{**metadata, 'response': response([line('Invented verse', end=2)])},
                       {**metadata, 'response': response([])}]
            with patch.object(acp, 'discover', return_value=dict(server=server, harness=harness, profile=root)), \
                 patch.object(acp, 'ask', new=AsyncMock(side_effect=answers)) as ask, \
                 patch.object(lyrics.subprocess, 'check_output', return_value=b'wav'):
                value = await lyrics.transcribe(audio, root / 'out.json', length=12,
                    model=acp.MODELS[0], confirmed=True)
            self.assertEqual(value['lines'], [])
            self.assertEqual(value['source']['unconfirmed_phrases'][0]['text'], 'Invented verse')
            self.assertEqual(ask.await_count, 2)
            self.assertNotIn('Invented verse', ask.call_args.args[2])

    async def test_no_consent_means_no_discovery_or_process(self):
        with patch.object(acp, 'discover') as discover:
            with self.assertRaisesRegex(ValueError, 'confirmation'):
                await lyrics.transcribe(Path('not-opened'), Path('not-written'), length=12,
                                        model=acp.MODELS[0], confirmed=False)
            discover.assert_not_called()

    async def test_clip_cache_reuses_exact_audio_prompt_runtime_and_model_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            audio = root / 'audio'; audio.write_bytes(b'audio')
            server = root / 'server'; server.write_bytes(b'runtime')
            harness = root / 'harness'; harness.write_bytes(b'harness')
            result = dict(response=response([line(end=2)]), agent={'name': 'antigravity-acp'},
                          model=acp.MODELS[0], session_id='test')
            with patch.object(acp, 'discover', return_value=dict(server=server, harness=harness, profile=root)), \
                 patch.object(acp, 'ask', new=AsyncMock(return_value=result)) as ask, \
                 patch.object(lyrics.subprocess, 'check_output', return_value=b'wav'):
                async def run(**kwargs):
                    return await lyrics.transcribe(audio, root / 'out.json', length=12,
                        model=acp.MODELS[0], confirmed=True, **kwargs)
                await run(); await run()
                self.assertEqual(ask.await_count, 2)
                local_reference = {'text': 'Private authored sheet canary', 'sha256': 'test', 'source': 'test'}
                value = await run(reference=local_reference)
                self.assertEqual(ask.await_count, 2)
                self.assertEqual(value['source']['reference'], local_reference)
                self.assertEqual(value['source']['reference_scope'], 'local-only')
                self.assertNotIn(local_reference['text'], ask.call_args.args[2])
                server.write_bytes(b'new runtime')
                await run()
                self.assertEqual(ask.await_count, 4)
                audio.write_bytes(b'new audio')
                await run()
                self.assertEqual(ask.await_count, 6)

    def test_snapshot_cannot_disguise_audio_as_local_or_unconfirmed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'snapshot.json'
            entry = dict(contract='TC-COARSE', route_type='antigravity',
                         runtime_id='antigravity-acp', model_id=acp.MODELS[0],
                         boundary_applied='audio-leaves-machine', boundary_confirmed=True)
            for changed in ({'boundary_confirmed': False}, {'boundary_applied': 'local-only'},
                            {'model_id': 'silently-substituted-model'}):
                path.write_text(json.dumps(dict(snapshot_schema=external_analysis.EXECUTION_SNAPSHOT_SCHEMA,
                                               contracts=[{**entry, **changed}])))
                with self.assertRaises(ValueError):
                    external_analysis.read_execution_snapshot(path)


class Discovery(unittest.TestCase):
    def test_explicit_missing_runtime_does_not_fall_back(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, 'not executable'):
                acp.discover(server=Path(directory) / 'missing', profile=directory,
                             environ={'HOME': directory})

    def test_multiple_accounts_are_not_guessed_and_token_contents_are_not_read(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            server = home / 'agy'; server.write_text('test'); server.chmod(0o700)
            harness = home / 'localharness_external'; harness.write_text('test'); harness.chmod(0o700)
            for name in ('a', 'b'):
                token = home / '.t3/userdata/providers/antigravity' / name / 'antigravity-acp/acp_token.json'
                token.parent.mkdir(parents=True); token.write_text('not even JSON')
            with self.assertRaisesRegex(ValueError, 'exactly one'):
                acp.discover(server=server, environ={'HOME': str(home)})
            profile = home / '.t3/userdata/providers/antigravity/a'
            result = acp.discover(server=server, profile=profile, environ={'HOME': str(home)})
            self.assertEqual(result['profile'], profile)

    def test_other_provider_credentials_do_not_reach_acp(self):
        with patch.dict(os.environ, {'OPENROUTER_API_KEY': 'canary', 'GOOGLE_API_KEY': 'canary',
                                    'RANDOM_TOKEN': 'canary', 'PATH': '/bin'}, clear=True):
            env = acp.child_environment(Path('/profile'), Path('/harness'))
        self.assertNotIn('canary', env.values())
        self.assertEqual(env['GEMINI_HOME'], '/profile')
        self.assertEqual(env['BROWSER'], '/bin/true')


class Transport(unittest.IsolatedAsyncioTestCase):
    def test_quota_and_empty_completion_are_transport_failures_not_json_retries(self):
        result = dict(stopReason='end_turn')
        with self.assertRaisesRegex(RuntimeError, 'reset in 56 minutes, 5 seconds'):
            acp.completed_response(result, ['Usage Limit Reached\n\nYou have reached your current quota for this period. Your limit will reset in 56 minutes, 5 seconds.'])
        with self.assertRaisesRegex(RuntimeError, 'without a response'):
            acp.completed_response(result, ['  '])
        self.assertEqual(acp.completed_response(result, ['{"lines":[]}']), '{"lines":[]}')
        self.assertEqual(acp.completed_response(result, ['{"lines":[{"text":"Usage Limit Reached quota"}]}']),
                         '{"lines":[{"text":"Usage Limit Reached quota"}]}')

    async def test_timeout_reaps_the_owned_acp_process_and_names_the_repair(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / 'stalled-acp'
            server.write_text('#!/usr/bin/env python3\nimport time\ntime.sleep(30)\n')
            server.chmod(0o700)
            args = argparse.Namespace(server=server, harness=server, profile=root,
                                      model=acp.MODELS[0], timeout=0.1)
            children = []
            create = asyncio.create_subprocess_exec
            async def launch(*args, **kwargs):
                child = await create(*args, **kwargs)
                children.append(child)
                return child
            with patch.object(acp.asyncio, 'create_subprocess_exec', side_effect=launch):
                with self.assertRaisesRegex(RuntimeError, 'resume its completed clips'):
                    await acp.ask(args, b'wav', 'Listen')
            self.assertEqual(len(children), 1)
            self.assertIsNotNone(children[0].returncode)

    async def test_official_protocol_audio_model_selection_and_tool_denial(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / 'fake-acp'
            server.write_text('''#!/usr/bin/env python3
import sys,json
sid='independent-context'
def send(v): print(json.dumps(v),flush=True)
for raw in sys.stdin:
 r=json.loads(raw)
 if 'method' not in r:
  assert r['result']['outcome']['outcome']=='cancelled'
  send({'method':'session/update','params':{'sessionId':sid,'update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'{"lines":[]}'}}}})
  send({'id':prompt_id,'result':{'stopReason':'end_turn'}})
  continue
 m,p=r['method'],r['params']
 if m=='initialize':
  assert p['clientCapabilities']['terminal'] is False
  result={'agentInfo':{'name':'antigravity-acp','version':'test'},'agentCapabilities':{'promptCapabilities':{'audio':True}}}
 elif m=='authenticate': result={}
 elif m=='session/new': result={'sessionId':sid,'models':{'availableModels':[{'modelId':'gemini-3.8-flash-high'}]}}
 elif m=='session/set_config_option':
  assert p['value']=='gemini-3.8-flash-high'
  send({'method':'session/update','params':{'sessionId':sid,'update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'startup notification'}}}})
  result={'configOptions':[{'id':'model','currentValue':p['value']}]}
 elif m=='session/prompt':
  assert p['prompt'][1]=={'type':'audio','mimeType':'audio/wav','data':'d2F2'}
  prompt_id=r['id']
  send({'id':99,'method':'session/request_permission','params':{}})
  continue
 send({'id':r['id'],'result':result})
''')
            server.chmod(0o700)
            args = argparse.Namespace(server=server, harness=server, profile=root,
                                      model=acp.MODELS[0], timeout=5)
            report = await acp.ask(args, b'wav', 'Listen')
            self.assertEqual(report['response'], '{"lines":[]}')
            self.assertIn({'denied_tool': 'session/request_permission'}, report['events'])
            args.model = 'not-offered'
            with self.assertRaisesRegex(RuntimeError, 'unavailable'):
                await acp.ask(args, b'wav', 'Listen')


if __name__ == '__main__':
    unittest.main()
