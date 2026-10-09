import fs from 'node:fs';
import http from 'node:http';
import assert from 'node:assert/strict';
const dir='/tmp/lens-final-removal-audit';const env=JSON.parse(fs.readFileSync('/tmp/lens-final-acceptance/environment.json'));
const startedAt=new Date().toISOString(),checks=[];let cookie='';
function record(label,expected,observed){checks.push({label,expected,observed,passed:expected===observed});fs.writeFileSync(dir+'/http-session-static.json',JSON.stringify({started_at:startedAt,updated_at:new Date().toISOString(),scope:'Own temporary standalone session only; actual raw HTTP static traversal and header-duplication requests; no ingestion/provider calls',checks},null,2)+'\n');assert.equal(observed,expected,label);}
function request(port,path,{method='GET',headers={},body}={}){return new Promise((resolve,reject)=>{const data=body===undefined?undefined:JSON.stringify(body);const req=http.request({hostname:'127.0.0.1',port,path,method,headers:{...headers,...(data===undefined?{}:{'content-type':'application/json','content-length':Buffer.byteLength(data)})}},res=>{const chunks=[];let bytes=0;res.on('data',chunk=>{bytes+=chunk.length;if(bytes>1024*1024){req.destroy();reject(new Error('Audit response exceeds1MiB'));return;}chunks.push(chunk);});res.on('end',()=>resolve({status:res.statusCode,headers:res.headers,body:Buffer.concat(chunks)}));});req.setTimeout(15000,()=>req.destroy(new Error('Audit HTTP timeout')));req.on('error',reject);req.end(data);});}
try {
 const signIn=await request(4326,'/auth/session',{method:'POST',body:{token:env.LENS_ADMIN_TOKEN}});record('Standalone setup credential creates session',200,signIn.status);const fullCookie=signIn.headers['set-cookie']?.[0];assert(fullCookie);cookie=fullCookie.split(';')[0];record('Session cookie is HttpOnly',true,/;\s*HttpOnly(?:;|$)/i.test(fullCookie));record('Session cookie uses strict same-site policy',true,/;\s*SameSite=strict(?:;|$)/i.test(fullCookie));
 record('Own session reads authenticated session info',200,(await request(4326,'/auth/session',{headers:{cookie}})).status);
 record('Cookie mutation without Origin is rejected',403,(await request(4326,'/auth/session',{method:'DELETE',headers:{cookie}})).status);
 record('Cookie mutation with foreign Origin is rejected',403,(await request(4326,'/auth/session',{method:'DELETE',headers:{cookie,origin:'https://audit-foreign.invalid'}})).status);
 record('Duplicate conflicting Origin headers are rejected',403,(await request(4326,'/auth/session',{method:'DELETE',headers:{cookie,origin:['https://audit-foreign.invalid','http://127.0.0.1:4326']}})).status);
 record('Invalid explicit bearer cannot fall back to a valid cookie',401,(await request(4326,'/auth/session',{headers:{cookie,authorization:'Bearer audit-invalid'}})).status);
 record('Own session survives rejected CSRF attempts',200,(await request(4326,'/auth/session',{headers:{cookie}})).status);
 record('Cookie logout with exact configured Origin succeeds',204,(await request(4326,'/auth/session',{method:'DELETE',headers:{cookie,origin:'http://127.0.0.1:4326'}})).status);
 record('Revoked standalone session is rejected immediately',401,(await request(4326,'/auth/session',{headers:{cookie}})).status);
}finally{if(cookie){const cleanup=await request(4326,'/auth/session',{method:'DELETE',headers:{cookie,origin:'http://127.0.0.1:4326'}});record('Temporary standalone session cleanup completed',true,[204,401].includes(cleanup.status));}}
const index=await request(4326,'/ui/');record('Bundled UI is served',200,index.status);record('Bundled UI has nosniff',true,index.headers['x-content-type-options']==='nosniff');
for(const path of ['/ui/../etc/passwd','/ui/%2e%2e/etc/passwd','/ui/_next/../../etc/passwd','/ui/%2e%2e%2fetc%2fpasswd','/ui/%252e%252e/etc/passwd','/ui/audit-missing.js','/auth/audit-missing']){const result=await request(4326,path);record('Static/API missing or traversal path rejects '+path,404,result.status);record('Missing/traversal response is not the UI shell '+path,false,result.body.equals(index.body));}
for(const [port,token] of [[4326,env.LENS_ADMIN_TOKEN],[4494,env.LENS_HOST_MASTER_KEY]]){
 record('Single supported eval contract accepted on '+port,200,(await request(port,'/lens/evals',{headers:{authorization:'Bearer '+token,'x-lens-contract':'1'}})).status);
 for(const values of [['1','1'],['2','1']])record('Duplicate raw eval contract headers rejected on '+port+':'+values.join('/'),409,(await request(port,'/lens/evals',{headers:{authorization:'Bearer '+token,'x-lens-contract':values}})).status);
}
console.log(JSON.stringify({passed:checks.every(c=>c.passed),checks:checks.length,receipt:dir+'/http-session-static.json'}));
