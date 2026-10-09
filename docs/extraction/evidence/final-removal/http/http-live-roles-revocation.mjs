import fs from 'node:fs';
import {execFileSync} from 'node:child_process';

const directory = '/tmp/lens-final-acceptance';
const env = JSON.parse(fs.readFileSync(directory + '/environment.json'));
const users = Object.fromEntries(['owner', 'viewer', 'other'].map(role => [role, JSON.parse(fs.readFileSync('/tmp/lens-host-live/user-' + role + '.json'))]));
const trace = JSON.parse(fs.readFileSync(directory + '/auth-proof.json')).trace_id;
const checks = [];
const outputDirectory = '/tmp/lens-final-removal-audit';
const receipt = {
  audit_start: '2026-10-09T11:59:21Z',
  fixture_scope: 'Existing traces only; temporary gateway key is created and revoked; no ingestion, paid calls or service changes',
  created_at: new Date().toISOString(),
  gateway_source: execFileSync('git', ['rev-parse', 'HEAD'], {cwd:'/Users/mfkhalil/Code/litellm-lens-host', encoding:'utf8'}).trim(),
  gateway_production_source: 'ba4cc3ded3aa805eac1808a8ae3eff7271c425db',
  lens_source: execFileSync('docker', ['image', 'inspect', '--format', '{{index .Config.Labels \"org.opencontainers.image.revision\"}}', execFileSync('docker', ['inspect', '--format', '{{.Image}}', 'lens-final-acceptance'], {encoding:'utf8'}).trim()], {encoding:'utf8'}).trim(),
  lens_image_id: execFileSync('docker', ['inspect', '--format', '{{.Image}}', 'lens-final-acceptance'], {encoding:'utf8'}).trim(),
  trace_id: trace,
  checks,
};
function check(label, expected, observed) {
  checks.push({label, expected, observed, passed: expected === observed});
  fs.writeFileSync(outputDirectory + '/http-live-roles-revocation.json', JSON.stringify(receipt, null, 2) + '\n');
  if (expected !== observed) throw new Error(label + ' failed: ' + observed);
}
async function request(path, token, method = 'GET', body) {
  const response = await fetch('http://127.0.0.1:4494' + path, {
    method,
    headers: {authorization:'Bearer ' + token, ...(body === undefined ? {} : {'content-type':'application/json'})},
    ...(body === undefined ? {} : {body: JSON.stringify(body)}),
    signal: AbortSignal.timeout(15000),
  });
  const text = await response.text();
  let json; try {json = JSON.parse(text)} catch {json = null}
  return {status: response.status, json};
}
for (const [role, token, traceStatus, productStatus] of [
  ['admin', env.LENS_HOST_MASTER_KEY, 200, 200],
  ['viewer', users.viewer.key, 200, 200],
  ['owner', users.owner.key, 200, 403],
  ['other', users.other.key, 404, 403],
]) {
  const traceRead = await request('/v1/traces/' + trace, token);
  check(role + ' trace access stays scoped', traceStatus, traceRead.status);
  if (traceStatus === 200) check(role + ' reads the stored trace', trace, traceRead.json?.summary?.trace_id);
  const productRead = await request('/lens/signals', token);
  check(role + ' Lens product role is preserved', productStatus, productRead.status);
  const serviceRead = await request('/lens/service', token);
  check(role + ' can read authenticated setup status', 200, serviceRead.status);
  check(role + ' sees connected storage', true, serviceRead.json?.connected && serviceRead.json?.status?.storage_ready);
  if (role !== 'admin') {
    const productWrite = await request('/lens/tracing/keys', token, 'POST', {name:'Rejected adapter requalification key'});
    check(role + ' cannot perform Lens admin mutation', 403, productWrite.status);
  }
}
const generated = await request('/key/generate', env.LENS_HOST_MASTER_KEY, 'POST', {user_id:users.viewer.user_id, key_alias:'lens-removal-audit-revocation', max_budget:1});
check('Temporary gateway key is created', 200, generated.status);
const key = generated.json?.key;
if (typeof key !== 'string') throw new Error('Temporary key missing');
try {
  const before = await request('/lens/signals', key);
  check('Temporary key reads Lens before revocation', 200, before.status);
  const revoked = await request('/key/delete', env.LENS_HOST_MASTER_KEY, 'POST', {keys:[key]});
  check('Temporary gateway key is revoked', 200, revoked.status);
  const after = await request('/lens/signals', key);
  check('Revocation denies Lens immediately', 401, after.status);
  const setupAfter = await request('/lens/service', key);
  check('Revocation denies setup immediately', 401, setupAfter.status);
} finally {
  await request('/key/delete', env.LENS_HOST_MASTER_KEY, 'POST', {keys:[key]});
}
console.log(JSON.stringify({checks:checks.length, passed:checks.every(c=>c.passed), receipt:outputDirectory + '/http-live-roles-revocation.json'}));
