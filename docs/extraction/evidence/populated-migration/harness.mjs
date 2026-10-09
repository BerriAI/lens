import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {openSync} from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import {spawn} from 'node:child_process';
import {createHash, randomBytes} from 'node:crypto';

const [phase, directory] = process.argv.slice(2);
assert.ok(['prepare', 'analyze', 'backup', 'restore', 'cleanup'].includes(phase), 'Use prepare|analyze|backup|restore|cleanup DIRECTORY');
assert.ok(directory?.startsWith('/tmp/lens-populated-'), 'Use a new /tmp/lens-populated-* directory');
const source = '/Users/mfkhalil/Code/litellm-lens';
const host = '/Users/mfkhalil/Code/litellm-lens-host';
const image = 'sha256:9a2eba30f01720fe66a6f45bcc3beba63caac2d3e44e59445bf1ccede7715563';
const input = '/tmp/lens-final-acceptance/environment.json';
const gatewayInput = '/tmp/lens-final-acceptance/gateway.json';
const privateFile = path.join(directory, 'private.json');
const proofFile = path.join(directory, 'proof.json');
const secret = () => randomBytes(32).toString('hex');
const sha = text => createHash('sha256').update(text).digest('hex');
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const json = async file => JSON.parse(await fs.readFile(file, 'utf8'));
async function save(file, value) { await fs.writeFile(file, JSON.stringify(value, null, 2)+'\n', {mode:0o600}); }
async function port() {
  const listener = net.createServer();
  await new Promise((resolve, reject) => { listener.once('error', reject); listener.listen(0, '127.0.0.1', resolve); });
  const number = listener.address().port;
  await new Promise(resolve => listener.close(resolve));
  return number;
}
let sequence=0;
async function command(executable, args, env={}, stdin) {
  const logFile=path.join(directory, `command-${Date.now()}-${sequence++}.log`);
  const child=spawn(executable,args,{env:{...process.env,...env},stdio:['pipe','pipe','pipe']});
  let stdout='',stderr='';
  child.stdout.on('data',chunk=>{stdout+=chunk;}); child.stderr.on('data',chunk=>{stderr+=chunk;});
  if(stdin!==undefined) child.stdin.end(stdin); else child.stdin.end();
  const code=await new Promise((resolve,reject)=>{child.on('error',reject);child.on('close',resolve);});
  await fs.writeFile(logFile,stdout+stderr,{mode:0o600});
  assert.equal(code,0,`${executable} failed; inspect private log ${logFile}`);
  return stdout;
}
function deployment(state, name) { return state[name]; }
async function compose(state,name,...args) {
  const d=deployment(state,name);
  return command('docker',['compose','--project-name',d.project,'--project-directory',d.directory,'-f',path.join(d.directory,'compose.yaml'),...args],{
    COMPOSE_PROJECT_NAME:d.project,LENS_PORT:String(d.port),LENS_PUBLIC_URL:`http://127.0.0.1:${d.port}`,
  });
}
async function http(base, endpoint, token, method='GET', body, expected=200, raw=false) {
  const response=await fetch(base+endpoint,{method,headers:{authorization:`Bearer ${token}`,'content-type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(30000)});
  const text=await response.text();
  assert.equal(response.status,expected,`${method} ${endpoint.split('?')[0]}: status ${response.status}`);
  if(raw)return text;
  try {return JSON.parse(text);} catch {return text;}
}
async function ready(base, endpoint='/health/ready') {
  for(let attempt=0;attempt<90;attempt++) {
    try {if((await fetch(base+endpoint,{signal:AbortSignal.timeout(3000)})).ok)return;}catch{}
    await delay(2000);
  }
  throw new Error(`Readiness timed out for ${base}${endpoint}`);
}
async function ch(state, query, rows) {
  const arguments_=['compose','--project-name',state.source.project,'--project-directory',state.source.directory,'-f',path.join(state.source.directory,'compose.yaml'),'exec','-T','clickhouse','sh','-c','exec clickhouse-client --user lens --password "$CLICKHOUSE_PASSWORD" --multiquery --query "$1"','sh',query];
  return command('docker',arguments_,{},rows);
}
async function gatewayBoundary() {
  const configured=await json(input);const url=new URL(configured.DATABASE_URL);
  assert.ok(['localhost','127.0.0.1','host.docker.internal'].includes(url.hostname),'Gateway sentinel must use the local task database');
  const schema=url.searchParams.get('schema')??'public';assert.match(schema,/^[A-Za-z_][A-Za-z0-9_]*$/);
  const query=`SELECT json_build_object(
    'schema_tables', (SELECT count(*) FROM pg_tables WHERE schemaname=current_schema()),
    'schema_hash', (SELECT md5(coalesce(string_agg(table_name || ':' || column_name || ':' || data_type || ':' || is_nullable, ',' ORDER BY table_name,ordinal_position),'')) FROM information_schema.columns WHERE table_schema=current_schema() AND table_name IN (SELECT tablename FROM pg_tables WHERE schemaname=current_schema())),
    'teams', (SELECT count(*) FROM "LiteLLM_TeamTable"),
    'team_configuration_hash', (SELECT md5(coalesce(string_agg(json_build_array(team_id,team_alias,models)::text, ',' ORDER BY team_id),'')) FROM "LiteLLM_TeamTable")
  );`;
  return JSON.parse(await command('/opt/homebrew/bin/psql',['--no-psqlrc','-qAtX','--set=ON_ERROR_STOP=on','-c',query],{PGHOST:url.hostname,PGPORT:url.port||'5432',PGUSER:decodeURIComponent(url.username),PGPASSWORD:decodeURIComponent(url.password),PGDATABASE:decodeURIComponent(url.pathname.slice(1)),PGOPTIONS:`-c default_transaction_read_only=on -c statement_timeout=10000 -c search_path=${schema}`}));
}
async function writeEnvironment(state) {
  const values={COMPOSE_PROJECT_NAME:state.source.project,LENS_IMAGE:image,LENS_ADMIN_TOKEN:state.admin,CLICKHOUSE_PASSWORD:state.clickhousePassword,LENS_PUBLIC_URL:state.source.origin,LENS_PORT:state.source.port,LENS_GATEWAY_SECRET:state.gatewaySecret,LITELLM_LENS_SERVICE_TOKEN:state.serviceToken,LENS_GATEWAY_URL:`http://host.docker.internal:${state.gatewayPort}/v1`,LENS_MIGRATION_ANALYSIS_KEY:state.analysisKey??secret(),LENS_ANALYSIS_MODELS:JSON.stringify([{...state.model,name:'analysis',api_key_env:'LENS_MIGRATION_ANALYSIS_KEY',api_base:`http://host.docker.internal:${state.gatewayPort}/v1`}])};
  await fs.writeFile(path.join(state.source.directory,'.env'),Object.entries(values).map(([key,value])=>`${key}=${value}`).join('\n')+'\n',{mode:0o600});
}
async function gateway(state, target='source') {
  const existing=await json(input);
  const config=await json(gatewayInput);
  config.general_settings={...config.general_settings,master_key:'os.environ/LENS_REHEARSAL_GATEWAY_MASTER',disable_prisma_schema_update:true};
  const configFile=path.join(directory,'gateway.json');
  await save(configFile,config);
  const env={...process.env,...existing,PYTHONPATH:host,LITELLM_MODE:'PRODUCTION',LITELLM_LOCAL_MODEL_COST_MAP:'True',LENS_REHEARSAL_GATEWAY_MASTER:state.gatewayMaster,LITELLM_MASTER_KEY:state.gatewayMaster,LENS_GATEWAY_SECRET:state.gatewaySecret,LITELLM_LENS_SERVICE_TOKEN:state.serviceToken,LITELLM_LENS_URL:state[target].origin,LITELLM_LENS_PUBLIC_URL:state[target].origin,PROXY_BASE_URL:state.gatewayOrigin,UI_USERNAME:'migration-admin',UI_PASSWORD:state.uiPassword,LITELLM_UI_PATH:path.join(host,'ui/litellm-dashboard/out'),DISABLE_UPDATE_CHECK:'True'};
  for(const name of Object.keys(env))if(name.startsWith('REDIS_')||['STORE_MODEL_IN_DB','LITELLM_URL'].includes(name))delete env[name];
  const log=openSync(path.join(directory,`gateway-${target}.log`),'a',0o600);
  const child=spawn(path.join(host,'.venv/bin/python'),['litellm/proxy/proxy_cli.py','--config',configFile,'--host','127.0.0.1','--port',String(state.gatewayPort)],{cwd:host,env,stdio:['ignore',log,log],detached:true});
  child.unref(); state.gatewayPid=child.pid; await save(privateFile,state);
  await ready(state.gatewayOrigin,'/health/liveliness');
}
async function stopGateway(state) {
  if(!state.gatewayPid)return;
  let identity;try{identity=await command('ps',['-p',String(state.gatewayPid),'-o','command=']);}catch{delete state.gatewayPid;await save(privateFile,state);return;}
  assert.ok(identity.includes(path.join(directory,'gateway.json'))&&identity.includes('--port '+state.gatewayPort),'Refusing to stop a gateway PID with different ownership');
  try{process.kill(-state.gatewayPid,'SIGTERM');}catch(error){if(error.code!=='ESRCH')throw error;}
  await delay(2500); delete state.gatewayPid; await save(privateFile,state);
}
async function retained(state,name) {
  const origin=state[name].origin;
  const investigation=await http(origin,'/lens/migrated-lens',state.admin);
  assert.equal(investigation.id,'migrated-lens');
  assert.ok(investigation.findings.some(finding=>finding.id==='migrated-finding'));
  assert.equal(investigation.scope.team_id,state.team);
  assert.ok(investigation.findings.find(finding=>finding.id==='migrated-finding').merged_finding_ids.includes('migrated-old-alias'));
  const archived=await http(origin,'/lens/migrated-lens/runs/migrated-archive',state.admin);
  assert.equal(archived.id,'migrated-archive');
  const historic=await http(origin,'/lens/datasets/migrated-dataset?revision=1',state.admin);
  const latest=await http(origin,'/lens/datasets/migrated-dataset',state.admin);
  assert.equal(historic.revision,1); assert.equal(latest.revision,2);
  assert.equal(historic.cases[0].expected,'Order found, revision one');
  assert.equal(latest.cases[0].expected,'Order found, revision two');
  const export1=await http(origin,'/lens/datasets/migrated-dataset/export?revision=1',state.admin,'GET',undefined,200,true);
  const export2=await http(origin,'/lens/datasets/migrated-dataset/export',state.admin,'GET',undefined,200,true);
  assert.notEqual(export1,export2);
  const trace=await http(origin,`/v1/traces/${state.traceId}`,state.admin);
  const feedback=await http(origin,'/lens/feedback?'+new URLSearchParams({trace_id:state.traceId,trace_ref:trace.summary.trace_ref}),state.admin);
  assert.equal(feedback.feedback[0].comment,'Preserved before PostgreSQL removal');
  const keys=await http(origin,'/lens/tracing/keys',state.admin);
  assert.ok(JSON.stringify(keys).includes(state.ingestionHash));
  const receipt=await http(origin,'/v1/traces/receipt',state.ingestionKey,'POST',{trace_id:state.traceId,span_ids:[state.rootSpan,state.toolSpan]});
  assert.equal(receipt.received,true);
  return {investigation,archived,historic,latest,export1,export2,trace,feedback,keys};
}
async function crossView(state,name) {
  const standalone=await retained(state,name);
  for(const endpoint of ['/lens/migrated-lens','/lens/migrated-lens/runs/migrated-archive','/lens/datasets/migrated-dataset?revision=1','/lens/datasets/migrated-dataset','/lens/datasets/migrated-dataset/export',`/v1/traces/${state.traceId}`]) {
    assert.deepEqual(await http(state.gatewayOrigin,endpoint,state.gatewayMaster),await http(state[name].origin,endpoint,state.admin),`Embedded API mismatch: ${endpoint}`);
  }
  return {dataset_revision:standalone.latest.revision,historical_export_sha256:sha(standalone.export1),current_export_sha256:sha(standalone.export2),trace_id:state.traceId,feedback_preserved:true,imported_key_receipt:true,standalone_embedded_api_equal:true};
}
async function prepare() {
  await fs.mkdir(directory,{mode:0o700});
  const sourcePort=await port(),restorePort=await port(),gatewayPort=await port();
  assert.equal(new Set([sourcePort,restorePort,gatewayPort]).size,3,'Port allocation collided; retry prepare in a fresh directory');
  const project=`lens-populated-${process.pid}-${randomBytes(3).toString('hex')}`;
  const existing=await json(input);
  assert.ok(existing.DATABASE_URL,'Existing task gateway test database config is required');
  assert.ok(['localhost','127.0.0.1','host.docker.internal'].includes(new URL(existing.DATABASE_URL).hostname),'Only the existing local task gateway database may be reused');
  const models=JSON.parse(existing.LENS_ANALYSIS_MODELS);
  assert.equal(models.length,1,'Select one existing qualified analysis deployment');
  const state={image,sourceCommit:'105e7b0923a17311e527aad4d480b57bab431000',model:models[0],admin:secret(),clickhousePassword:secret(),gatewaySecret:secret(),serviceToken:secret(),gatewayMaster:'sk-'+secret(),uiPassword:secret(),ingestionKey:'lens-migration-'+secret(),team:project,traceId:randomBytes(16).toString('hex'),rootSpan:randomBytes(8).toString('hex'),toolSpan:randomBytes(8).toString('hex'),gatewayPort,gatewayOrigin:`http://127.0.0.1:${gatewayPort}`,postgres:`${project}-postgres`,source:{directory:path.join(directory,'source/deploy/lens'),project:`${project}-source`,port:sourcePort,origin:`http://127.0.0.1:${sourcePort}`},restored:{directory:path.join(directory,'restored/deploy/lens'),project:`${project}-restored`,port:restorePort,origin:`http://127.0.0.1:${restorePort}`}};
  state.ingestionHash=sha(state.ingestionKey);
  state.gatewayBoundaryBefore=await gatewayBoundary();
  state.hostCommit=(await command('git',['-C',host,'rev-parse','HEAD'])).trim();
  state.lensHead=(await command('git',['-C',source,'rev-parse','HEAD'])).trim();
  for(const owned of ['src/worker/crates/migrate','src/worker/crates/traces-clickhouse','deploy/lens','deploy/clickhouse'])assert.equal((await command('git',['-C',source,'rev-parse',state.sourceCommit+':'+owned])).trim(),(await command('git',['-C',source,'rev-parse','HEAD:'+owned])).trim(),'Review changed Lens fixture/deployment source before executing');
  await save(privateFile,state);
  for(const d of [state.source,state.restored]) {
    await fs.mkdir(d.directory,{recursive:true,mode:0o700}); await fs.mkdir(path.join(d.directory,'../clickhouse'),{recursive:true});
    for(const name of ['start','backup','restore','compose.yaml'])await fs.copyFile(path.join(source,'deploy/lens',name),path.join(d.directory,name));
    await fs.copyFile(path.join(source,'deploy/clickhouse/keeper.xml'),path.join(d.directory,'../clickhouse/keeper.xml'));
  }
  assert.equal((await command('docker',['image','inspect','--format','{{.Id}}',image])).trim(),image);
  await writeEnvironment(state);
  await compose(state,'source','up','--detach','--wait','--wait-timeout','180','clickhouse');
  await command('docker',['run','-d','--name',state.postgres,'--network',`${state.source.project}_storage`,'--memory','512m','--cpus','1','-e','POSTGRES_PASSWORD=fixture-postgres','postgres:16-alpine@sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea']);
  for(let attempt=0;attempt<60;attempt++){try{await command('docker',['exec',state.postgres,'pg_isready','-U','postgres']);break;}catch{assert.ok(attempt<59,'PostgreSQL readiness');await delay(1000);}}
  const migrationFolder=path.join(source,'src/worker/crates/traces-clickhouse/migrations');
  for(const file of (await fs.readdir(migrationFolder)).filter(name=>name.endsWith('.sql')).sort()) {
    const sql=(await fs.readFile(path.join(migrationFolder,file),'utf8')).replaceAll('{database}','lens').replaceAll('{retention_days}','14');
    await ch(state,sql);
  }
  const now=new Date(Date.now()-600000).toISOString(); state.traceStart=now; state.traceEnd=new Date(Date.now()-300000).toISOString();
  const timestamp=now.replace('T',' ').replace('Z','');
  const common={Timestamp:timestamp,TraceId:state.traceId,ServiceName:'migration-proof',TeamId:state.team,ApiKeyHash:state.ingestionHash,UserId:'migration-owner',ResourceAttributes:{'litellm.team_id':state.team,'litellm.api_key_hash':state.ingestionHash},StatusCode:'STATUS_CODE_OK',Duration:1000000000};
  const rows=[{...common,SpanId:state.rootSpan,ParentSpanId:'',SpanName:'Migrated invoice agent',ObservationType:'agent',AgentName:'migration-proof',Input:'[{"role":"user","content":"Has order 42 been paid?"}]',Output:'[{"role":"assistant","content":"Order 42 is fully paid."}]'},{...common,SpanId:state.toolSpan,ParentSpanId:state.rootSpan,SpanName:'lookup_order',ObservationType:'tool',AgentName:'migration-proof',Input:'{"order_id":42}',Output:'{"payment_received":false,"status":"unpaid"}'}];
  await ch(state,'INSERT INTO lens.otel_traces FORMAT JSONEachRow',rows.map(row=>JSON.stringify(row)).join('\n')+'\n');
  await ch(state,'INSERT INTO lens.lens_feedback FORMAT JSONEachRow',JSON.stringify({TeamId:state.team,ApiKeyHash:state.ingestionHash,TraceId:state.traceId,Author:'migration-reviewer',Score:3,Comment:'Preserved before PostgreSQL removal',CreatedAt:timestamp,UpdatedAt:timestamp,IsDeleted:0})+'\n');
  const fixture=await json(path.join(source,'src/worker/crates/contract/tests/fixtures/investigations_public.json'));
  const execution=Buffer.from(JSON.stringify(['traces',state.team,state.traceId])).toString('base64');
  const settings={...fixture.filled_lens.settings,name:'Migrated invoice investigation',enabled:false,model:'analysis',agent_name:'migration-proof',monthly_budget:1,concurrency:1,checks:[{id:'retries',instruction:'Find whether the final answer contradicts the order lookup payment status',enabled:true}]};
  const finding={...fixture.filled_lens.findings[0],id:'migrated-finding',title:'Preserved finding',merged_finding_ids:['migrated-old-alias'],evidence:[{execution_id:execution,span_id:state.toolSpan,quote:'unpaid',role:'support'}],occurrences:[execution]};
  const review={...fixture.review,execution_id:execution,trace_id:state.traceId,content_version:'preserved-import-review'};
  const sample={...fixture.job.sample,executions:[{...fixture.job.sample.executions[0],id:execution,trace_id:state.traceId,team_id:state.team,name:'Migrated invoice agent',start_time:timestamp,root_seen:true,service:'migration-proof'}]};
  const job={...fixture.job,id:'migrated-current',status:'completed',stage:'Completed',settings,created_at:now,start:now,end:state.traceEnd,finished_at:now,findings:[finding],reviews:[review],activities:[],sample};
  const lens={...fixture.filled_lens,id:'migrated-lens',scope:{team_id:state.team,api_key_hash:'',all_teams:false},settings,created_at:now,next_run_at:now,jobs:[job],findings:[finding],budget_month:now.slice(0,7)};
  const dataset={id:'migrated-dataset',name:'Migrated order cases',agent_name:'migration-proof',team_id:state.team,created_at:now,revision:1,created_by:'migration-owner',cases:[{id:'migrated-case',messages:[{role:'user',content:'Find order 42'}],reply:'Order found',tool_calls:[{name:'lookup_order',arguments:'{"order_id":42}'}],expected:'Order found, revision one',included:true,source:{trace_id:state.traceId,span_id:state.toolSpan,finding_id:finding.id,lens_id:lens.id},agent_version:'migration-v1'}]};
  const dataset2=structuredClone(dataset);dataset2.revision=2;dataset2.cases[0].expected='Order found, revision two';
  const pythonArray=value=>Array.isArray(value)?'['+value.map(pythonArray).join(', ')+']':JSON.stringify(value);
  const criteria=sha(pythonArray([settings.context.trim(),settings.checks.filter(check=>check.enabled).map(check=>[check.id,check.instruction.trim()]).sort(),settings.model]));
  const snapshot={lenses:[{id:lens.id,version:42,data:lens,due_at:null}],runs:[{id:'migrated-archive',lens_id:lens.id,created_at:now,data:{...job,id:'migrated-archive'}}],reviews:[{lens_id:lens.id,criteria_key:criteria,execution_id:execution,data:review}],workers:[{id:'migrated-worker',token_hash:'preserved-worker-token-hash',data:{...fixture.worker,id:'migrated-worker',scope:lens.scope,analysis_key_id:sha('preserved-gateway-key-hash')}}],ingestion_keys:[{id:state.ingestionHash,data:{id:state.ingestionHash,name:'Imported tracing key',tenant:{team_id:state.team,user_id:'migration-owner',org_id:'',api_key_hash:state.ingestionHash},created_at:now,expires_at:Math.floor(Date.now()/1000)+3600}}],datasets:[{id:dataset.id,revision:1,created_at:now,data:dataset},{id:dataset2.id,revision:2,created_at:now,data:dataset2}],signal_configs:[{id:'global',data:{}}],trace_signals:[{trace_id:state.traceId,trace_ref:'',config_key:'preserved-signal-config',span_count:2,claimed_until:null,classified_at:null,data:{status:'pending',scores:{},model:'analysis',error:''}}]};
  await save(path.join(directory,'source-snapshot.json'),snapshot);
  await command('docker',['exec','-i',state.postgres,'psql','-U','postgres','--set=ON_ERROR_STOP=on'],{},await fs.readFile(path.join(source,'src/worker/crates/migrate/tests/support/postgres.sql')));
  const tables={lenses:'LiteLLM_Lens',runs:'LiteLLM_LensRun',reviews:'LiteLLM_LensReview',workers:'LiteLLM_LensWorker',ingestion_keys:'LiteLLM_LensIngestionKey',datasets:'LiteLLM_LensDataset',signal_configs:'LiteLLM_LensSignalConfig',trace_signals:'LiteLLM_LensTraceSignal'};
  for(const [key,table] of Object.entries(tables))await command('docker',['exec','-i',state.postgres,'psql','-U','postgres','--set=ON_ERROR_STOP=on',`--set=rows=${JSON.stringify(snapshot[key])}`],{},`INSERT INTO "${table}" SELECT * FROM jsonb_populate_recordset(NULL::"${table}", :'rows'::jsonb);\n`);
  await fs.writeFile(path.join(directory,'postgres-before.sql'),await command('docker',['exec',state.postgres,'pg_dump','-U','postgres']),{mode:0o600});
  await compose(state,'source','stop','clickhouse');
  const storage=(await compose(state,'source','ps','--all','--quiet','clickhouse')).trim();
  await command('docker',['cp','--archive',`${storage}:/var/lib/clickhouse/.`,path.join(directory,'clickhouse-before')]);
  await compose(state,'source','up','--detach','--wait','--wait-timeout','180','clickhouse');
  const migrate=async(...args)=>JSON.parse(await compose(state,'source','run','--rm','--no-deps','-e',`LENS_MIGRATION_POSTGRES_URL=postgres://migration_reader:fixture-reader@${state.postgres}:5432/postgres`,'--entrypoint','/usr/local/bin/lens-migrate','lens',...args));
  const planned=await migrate(); assert.equal(planned.applied,false); assert.equal(Object.keys(planned.plan.source_rows).length,8);
  const started=Date.now(); const applied=await migrate('--apply','--source-stopped'); assert.equal(applied.verified,true); assert.deepEqual(planned.plan,applied.plan);
  assert.deepEqual(applied,await migrate('--apply','--source-stopped'));
  assert.deepEqual(planned,await migrate());
  assert.deepEqual(await gatewayBoundary(),state.gatewayBoundaryBefore,'Unrelated gateway schema/team configuration changed during Lens import');
  await command('docker',['stop',state.postgres]);
  assert.equal((await command('docker',['inspect','--format','{{.State.Running}}',state.postgres])).trim(),'false');
  await compose(state,'source','up','--detach','--wait','--wait-timeout','180'); await ready(state.source.origin);
  const startupMilliseconds=Date.now()-started;
  await gateway(state);
  await retained(state,'source');
  const key=await http(state.gatewayOrigin,'/key/generate',state.gatewayMaster,'POST',{key_alias:`${project}-restricted-analysis`,models:[state.model.model],max_budget:1,budget_duration:'1d',duration:'1h'});
  assert.ok(key.key); state.analysisKey=key.key; await save(privateFile,state);
  const info=await http(state.gatewayOrigin,'/key/info?key='+encodeURIComponent(state.analysisKey),state.gatewayMaster);
  assert.equal(info.info.max_budget,1); assert.deepEqual(info.info.models,[state.model.model]);
  await http(state.gatewayOrigin,'/v1/chat/completions',sha('preserved-gateway-key-hash'),'POST',{model:state.model.model,messages:[{role:'user',content:'Must not reach a provider'}]},401);
  await http(state.gatewayOrigin,'/v1/chat/completions',state.analysisKey,'POST',{model:'not-permitted-by-migration-key',messages:[{role:'user',content:'Must not reach a provider'}]},403);
  await writeEnvironment(state); await compose(state,'source','up','--detach','--force-recreate','--wait','--wait-timeout','180','lens'); await ready(state.source.origin);
  const readback=await crossView(state,'source');
  const login=await fetch(state.source.origin+'/auth/session',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({token:state.admin})}); assert.equal(login.status,200);state.cookie=login.headers.get('set-cookie').split(';')[0];
  await save(privateFile,state);
  await save(proofFile,{status:'prepared_not_complete',image,source_commit:state.sourceCommit,host_commit:state.hostCommit,lens_reviewed_head:state.lensHead,source_snapshot_sha256:sha(JSON.stringify(snapshot)),migration:applied,postgres_stopped:true,import_to_ready_ms:startupMilliseconds,readback,gateway_database:{before:state.gatewayBoundaryBefore,after_import:await gatewayBoundary(),allowed_new_writes:'Unique analysis key and its request spend are excluded from stable schema/team comparison'},restricted_analysis:{new_credential:true,model:state.model.model,max_budget:1,legacy_hash_rejected:true,unlisted_model_rejected:true},ui_evidence:'pending',analysis:'pending',restore:'pending'});
  console.log(JSON.stringify({status:'prepared',private_file:privateFile,proof_file:proofFile,standalone:state.source.origin+'/ui/',embedded:state.gatewayOrigin+'/ui/?page=lens',next:'Capture both UIs, then run analyze'}));
}
async function analyze(state) {
  const before=await http(state.source.origin,'/lens/migrated-lens',state.admin);
  const updated=await http(state.source.origin,'/lens/migrated-lens/runs',state.admin,'POST',{start:state.traceStart,end:state.traceEnd,settings:before.settings});
  const id=updated.jobs[0].id; assert.notEqual(id,'migrated-current');
  const deadline=Date.now()+900000;let job;
  while(Date.now()<deadline){job=await http(state.source.origin,`/lens/migrated-lens/runs/${id}`,state.admin);if(['completed','failed','cancelled'].includes(job.status))break;await delay(5000);}
  await save(path.join(directory,'analysis-job.json'),job);assert.equal(job.status,'completed','Inspect private analysis-job.json');assert.ok(job.cost>0,'Requires actual paid provider use');
  const current=await http(state.source.origin,'/lens/migrated-lens',state.admin);
  assert.ok(current.spent>before.spent);assert.equal(current.reservations.filter(row=>!row.expires_at||Date.parse(row.expires_at)>Date.now()).length,0);
  let info;for(let attempt=0;attempt<30;attempt++){info=await http(state.gatewayOrigin,'/key/info?key='+encodeURIComponent(state.analysisKey),state.gatewayMaster);if(info.info.spend>0)break;await delay(3000);}
  assert.ok(info.info.spend>0,'Gateway must attribute actual spend to the new restricted key');
  assert.ok(Math.abs(job.cost-info.info.spend)<1e-8,'Lens cost must match restricted gateway key spend');
  assert.ok(Math.abs((current.spent-before.spent)-job.cost)<1e-8,'Investigation spend delta must match completed job');
  const proof=await json(proofFile);proof.analysis={status:'passed',job_id:id,cost:job.cost,gateway_key_spend:info.info.spend,model:state.model.model,active_reservations:0,findings:job.findings?.length??0};await save(proofFile,proof);
  console.log(JSON.stringify({status:'analysis_passed',proof_file:proofFile,next:'Capture the completed migrated investigation, then run backup'}));
}
async function completedAnalysis(state,name,proof) {
  const job=await http(state[name].origin,`/lens/migrated-lens/runs/${proof.analysis.job_id}`,state.admin);
  const investigation=await http(state[name].origin,'/lens/migrated-lens',state.admin);
  assert.equal(job.status,'completed');assert.equal(job.cost,proof.analysis.cost);
  return {job_id:job.id,status:job.status,cost:job.cost,findings:job.findings.length,job_sha256:sha(JSON.stringify(job)),investigation_spent:investigation.spent};
}
async function backup(state) {
  const proof=await json(proofFile);assert.equal(proof.analysis?.status,'passed','Finish bounded analysis first');
  const started=Date.now();await command(path.join(state.source.directory,'backup'),[path.join(directory,'snapshot')],{COMPOSE_PROJECT_NAME:state.source.project,LENS_PORT:String(state.source.port),LENS_PUBLIC_URL:state.source.origin});await ready(state.source.origin);
  proof.backup={status:'passed',elapsed_ms:Date.now()-started,recovery_point:new Date().toISOString(),readback:await crossView(state,'source'),completed_analysis:await completedAnalysis(state,'source',proof)};
  const sentinel=await http(state.source.origin,'/lens/datasets',state.admin,'POST',{name:'After-backup sentinel',agent_name:'migration-proof'});state.sentinel=sentinel.id;
  await save(privateFile,state);await save(proofFile,proof);console.log(JSON.stringify({status:'backup_passed',proof_file:proofFile,next:'Run restore; original Lens and ClickHouse will be stopped'}));
}
async function restore(state) {
  const proof=await json(proofFile);assert.equal(proof.backup?.status,'passed');
  await compose(state,'source','stop');await stopGateway(state);
  const started=Date.now();await command(path.join(state.restored.directory,'restore'),[path.join(directory,'snapshot')],{COMPOSE_PROJECT_NAME:state.restored.project,LENS_PORT:String(state.restored.port),LENS_PUBLIC_URL:state.restored.origin});await ready(state.restored.origin);await gateway(state,'restored');
  const readback=await crossView(state,'restored');assert.deepEqual(readback,proof.backup.readback);
  const completed_analysis=await completedAnalysis(state,'restored',proof);assert.deepEqual(completed_analysis,proof.backup.completed_analysis);
  await http(state.restored.origin,`/lens/datasets/${state.sentinel}`,state.admin,'GET',undefined,404);
  assert.equal((await fetch(state.restored.origin+'/lens/datasets',{headers:{cookie:state.cookie},signal:AbortSignal.timeout(30000)})).status,200);
  assert.equal((await command('docker',['inspect','--format','{{.State.Running}}',state.postgres])).trim(),'false');
  const gatewayAfterRestore=await gatewayBoundary();assert.deepEqual(gatewayAfterRestore,state.gatewayBoundaryBefore,'Unrelated gateway schema/team configuration changed during Lens restore');proof.gateway_database.after_restore=gatewayAfterRestore;
  proof.restore={status:'passed',elapsed_ms:Date.now()-started,readback,completed_analysis,session_retained:true,post_backup_sentinel_absent:true,postgres_still_stopped:true,data_loss_expectation:'Writes after the cold snapshot are deliberately absent; unchanged legacy PostgreSQL remains available for pre-new-write rollback',downtime_expectation:'Cold backup and restore stop ingestion; elapsed operation timings bound the local rehearsal, not a production SLA'};proof.status='runtime_passed_browser_evidence_pending';await save(proofFile,proof);
  console.log(JSON.stringify({status:proof.status,proof_file:proofFile,restored_standalone:state.restored.origin+'/ui/',embedded:state.gatewayOrigin+'/ui/?page=lens',next:'Capture restored UI evidence and then cleanup'}));
}
async function cleanup(state) {
  if(state.analysisKey){try{await http(state.gatewayOrigin,'/key/delete',state.gatewayMaster,'POST',{keys:[state.analysisKey]});}catch{throw new Error('Revoke the task analysis key before removing its gateway; private credential remains in private.json');}}
  await stopGateway(state);
  for(const name of ['source','restored']){try{await fs.access(path.join(state[name].directory,'.env'));}catch{continue;}await compose(state,name,'down','--volumes','--remove-orphans');}
  try{await command('docker',['rm','-fv',state.postgres]);}catch{}
  console.log(JSON.stringify({status:'cleaned_task_resources',private_directory_retained:directory}));
}
function validateOwned(state){assert.equal(state.image,image);for(const name of ['source','restored']){assert.ok(state[name].project.startsWith('lens-populated-'));assert.equal(state[name].directory,path.join(directory,name,'deploy/lens'));assert.equal(state[name].origin,`http://127.0.0.1:${state[name].port}`);}assert.ok(state.postgres.startsWith('lens-populated-'));assert.equal(state.gatewayOrigin,`http://127.0.0.1:${state.gatewayPort}`);}
try{if(phase==='prepare')await prepare();else{const state=await json(privateFile);validateOwned(state);if(phase==='analyze')await analyze(state);if(phase==='backup')await backup(state);if(phase==='restore')await restore(state);if(phase==='cleanup')await cleanup(state);}}catch(error){console.error(JSON.stringify({status:'failed',phase,message:error.message,private_directory:directory}));process.exitCode=1;}
