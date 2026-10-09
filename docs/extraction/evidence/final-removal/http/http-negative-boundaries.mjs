import fs from 'node:fs';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
const dir='/tmp/lens-final-removal-audit';
const env=JSON.parse(fs.readFileSync('/tmp/lens-final-acceptance/environment.json'));
const users=Object.fromEntries(['owner','viewer','other'].map(role=>[role,JSON.parse(fs.readFileSync('/tmp/lens-host-live/user-'+role+'.json'))]));
const trace=JSON.parse(fs.readFileSync('/tmp/lens-final-acceptance/auth-proof.json')).trace_id;
const fixtureTeam=JSON.parse(fs.readFileSync('/tmp/lens-host-live/tracing-key.json')).record.tenant.team_id;
const checks=[];const startedAt=new Date().toISOString();
const b64=x=>Buffer.from(JSON.stringify(x)).toString('base64url');
function jwt(claims,key=env.LENS_GATEWAY_SECRET,alg='HS256') {const content=b64({typ:'JWT',alg})+'.'+b64(claims);return content+'.'+(alg==='none'?'':crypto.createHmac(alg==='HS512'?'sha512':'sha256',key).update(content).digest('base64url'));}
function claims(role='other') {const now=Math.floor(Date.now()/1000),user=users[role];return {iss:'litellm',aud:'litellm-lens',sub:user.user_id,iat:now,exp:now+30,identity:{user_role:role==='viewer'?'proxy_admin_viewer':'internal_user',user_id:user.user_id,team_id:null,org_id:null,token:null,models:[],log_team_ids:role==='owner'?[fixtureTeam]:[]}};}
async function check(label,base,path,expected,{token,method='GET',body,headers=[]}={}) {const response=await fetch(base+path,{method,headers:[...(token?[['authorization','Bearer '+token]]:[]),...(body===undefined?[]:[['content-type','application/json']]),...headers],...(body===undefined?{}:{body:JSON.stringify(body)}),signal:AbortSignal.timeout(15000)});await response.arrayBuffer();checks.push({label,method,path,expected,observed:response.status,passed:response.status===expected});fs.writeFileSync(dir+'/http-negative-boundaries.json',JSON.stringify({started_at:startedAt,updated_at:new Date().toISOString(),scope:'Stateless read/auth denials, existing trace only; no ingestion, paid inference or shared-service changes',checks},null,2)+'\n');assert.equal(response.status,expected,label);}
const lens='http://127.0.0.1:4326',host='http://127.0.0.1:4494';
await check('Valid delegated session accepted',lens,'/auth/session',200,{token:jwt(claims('owner'))});
await check('Signed owner log-team grant can read existing team trace',lens,'/v1/traces/'+trace,200,{token:jwt(claims('owner'))});
{const unscoped=claims('owner');unscoped.identity.log_team_ids=[];await check('Owner identity without log-team grant cannot read team trace',lens,'/v1/traces/'+trace,404,{token:jwt(unscoped)});}
await check('Other user cannot read existing trace directly',lens,'/v1/traces/'+trace,404,{token:jwt(claims())});
await check('Browser query cannot expand signed trace scope',lens,'/v1/traces/'+trace+'?all_teams=1&user_id='+encodeURIComponent(users.owner.user_id),404,{token:jwt(claims())});
await check('Browser headers cannot expand signed trace scope',lens,'/v1/traces/'+trace,404,{token:jwt(claims()),headers:[['x-user-role','proxy_admin'],['x-team-id',''] ]});
for(const [name,mutate] of [
 ['expired',c=>({...c,iat:c.iat-60,exp:c.iat-1})],['future',c=>({...c,iat:c.iat+30,exp:c.exp+30})],['excessive lifetime',c=>({...c,exp:c.iat+61})],['wrong issuer',c=>({...c,iss:'other'})],['wrong audience',c=>({...c,aud:'other'})],['subject mismatch',c=>({...c,sub:'other-subject'})],['unknown role',c=>({...c,identity:{...c.identity,user_role:'superadmin'}})],['unknown claim',c=>({...c,privileged:true})],['missing expiry',c=>{const {exp,...rest}=c;return rest;}],['unknown identity field',c=>({...c,identity:{...c.identity,admin:true}})]
]) await check('Gateway delegation rejects '+name,lens,'/auth/session',401,{token:jwt(mutate(claims()))});
await check('Gateway delegation rejects wrong signature',lens,'/auth/session',401,{token:jwt(claims(),'audit-invalid-signing-secret-long-enough')});
await check('Gateway delegation rejects alternate algorithm',lens,'/auth/session',401,{token:jwt(claims(),env.LENS_GATEWAY_SECRET,'HS512')});
await check('Gateway delegation rejects unsigned token',lens,'/auth/session',401,{token:jwt(claims(),env.LENS_GATEWAY_SECRET,'none')});
await check('Service credential cannot become browser identity',lens,'/auth/session',401,{token:env.LITELLM_LENS_SERVICE_TOKEN});
assert.notEqual(env.LENS_ADMIN_TOKEN,env.LITELLM_LENS_SERVICE_TOKEN);
await check('Admin credential cannot become service credential',lens,'/internal/status',401,{token:env.LENS_ADMIN_TOKEN});
await check('Service status accepts proper service credential',lens,'/internal/status',200,{token:env.LITELLM_LENS_SERVICE_TOKEN});
await check('Ordinary gateway credential cannot call Lens internals',lens,'/internal/status',401,{token:users.owner.key});
await check('Viewer cannot create ingestion key',lens,'/lens/tracing/keys',403,{token:jwt(claims('viewer')),method:'POST',body:{name:'Rejected audit key'}});
await check('Unauthenticated gateway trace read rejected',host,'/v1/traces/'+trace,401);
await check('Unauthenticated gateway Lens read rejected',host,'/lens/signals',401);
await check('Forged internal marker rejected before gateway service read',host,'/lens/service',401,{token:env.LENS_HOST_MASTER_KEY,headers:[['x-lens-internal','forged']]});
const now=Math.floor(Date.now()/1000),internal=jwt({iss:'litellm-lens',aud:'litellm',sub:'lens-internal',purpose:'analysis',iat:now,exp:now+30});
await check('Valid internal marker does not bypass gateway authentication',host,'/lens/service',401,{token:'audit-invalid-gateway-key',headers:[['x-lens-internal',internal]]});
await check('Duplicate internal markers rejected',host,'/lens/service',401,{token:env.LENS_HOST_MASTER_KEY,headers:[['x-lens-internal',internal],['x-lens-internal','forged']]});
await check('Internal marker never authorizes inference',host,'/v1/chat/completions',401,{token:'audit-invalid-gateway-key',method:'POST',body:{model:'audit-never-execute',messages:[]},headers:[['x-lens-internal',internal]]});
console.log(JSON.stringify({passed:checks.every(c=>c.passed),checks:checks.length,receipt:dir+'/http-negative-boundaries.json'}));
