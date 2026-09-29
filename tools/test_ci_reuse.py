"""Release reuse must never mix commits, branches, failed runs or expired files."""
import contextlib
import io
import os
import unittest
from unittest import mock

import ci_reuse

SHA = 'a' * 40
REPO = 'Team-Haruki/Haruki-Sekai-API'


def run(**changes):
    value = dict(id=123, head_sha=SHA, head_branch='main', event='push', status='completed', conclusion='success')
    value.update(changes)
    return value


class ReuseTests(unittest.TestCase):
    def request(self, runs=None, artifacts=None):
        def get(path):
            if '/artifacts?' in path:
                return {'artifacts': artifacts or []}
            return {'workflow_runs': runs or []}
        return get

    def test_reuses_exact_commit_and_main_push_only(self):
        source = self.request([run(id=999, head_sha='b' * 40), run(id=500, head_branch='other'), run(id=600, event='pull_request'), run()])
        self.assertEqual(ci_reuse.find_run(REPO, SHA, 'docker', source), 123)
        self.assertIsNone(ci_reuse.find_run(REPO, SHA, 'docker', self.request()))

    def test_rejects_invalid_inputs_before_api(self):
        for repo, sha, kind in [('../repo', SHA, 'docker'), (REPO, 'bad', 'release'), (REPO, SHA, '../bad')]:
            with self.subTest(repo=repo, sha=sha, kind=kind), self.assertRaises(ValueError):
                ci_reuse.find_run(repo, sha, kind, mock.Mock(side_effect=AssertionError))

    def test_reuses_only_complete_unexpired_artifact_set(self):
        items = [dict(name=n, expired=False) for n in ci_reuse.ARTIFACTS]
        self.assertEqual(ci_reuse.find_run(REPO, SHA, 'release', self.request([run()], items)), 123)
        items[0]['expired'] = True
        self.assertIsNone(ci_reuse.find_run(REPO, SHA, 'release', self.request([run()], items)))
        self.assertIsNone(ci_reuse.find_run(REPO, SHA, 'release', self.request([run()])))

    def test_failed_build_blocks_release_and_cancelled_build_falls_back(self):
        with self.assertRaises(RuntimeError):
            ci_reuse.find_run(REPO, SHA, 'docker', self.request([run(conclusion='failure')]))
        self.assertIsNone(ci_reuse.find_run(REPO, SHA, 'docker', self.request([run(conclusion='cancelled')])))

    def test_waits_for_running_build_and_has_a_deadline(self):
        request = mock.Mock(side_effect=[{'workflow_runs': [run(status='in_progress')]}, {'workflow_runs': [run()]}])
        sleep = mock.Mock()
        self.assertEqual(ci_reuse.find_run(REPO, SHA, 'docker', request, now=lambda: 0, wait=sleep), 123)
        sleep.assert_called_once_with(20)
        with self.assertRaises(TimeoutError):
            ci_reuse.find_run(REPO, SHA, 'docker', self.request([run(status='queued')]), now=mock.Mock(side_effect=[0, 1800]), wait=sleep)

    def test_api_uses_checked_bounded_subprocess(self):
        with mock.patch.object(ci_reuse.subprocess, 'run', return_value=mock.Mock(stdout='{"ok":true}')) as invoke:
            self.assertEqual(ci_reuse.api('repos/a/b/actions'), {'ok': True})
            invoke.assert_called_once_with(['gh', 'api', 'repos/a/b/actions'], check=True, capture_output=True, text=True, timeout=60)

    def test_emits_only_fixed_outputs(self):
        for value, expected in [(123, 'reuse=true\nrun_id=123\n'), (None, 'reuse=false\nrun_id=\n')]:
            output = io.StringIO()
            with mock.patch.dict(os.environ, {'GITHUB_REPOSITORY': REPO, 'GITHUB_SHA': SHA}), mock.patch('sys.argv', ['ci_reuse.py', 'release']), mock.patch.object(ci_reuse, 'find_run', return_value=value), contextlib.redirect_stdout(output):
                ci_reuse.main()
            self.assertEqual(output.getvalue(), expected)
