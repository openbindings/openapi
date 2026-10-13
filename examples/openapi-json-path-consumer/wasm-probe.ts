import { authenticated_call, authenticated_consumers, crud_consumers, exact_roundtrip, pending_cleanup, logical_owners } from './wasm/openapi_json_path_consumer';
// No source/input/output value is parsed into a JavaScript Number.
const description = '{"openapi":"3.1.2","info":{"title":"Node","version":"1"},"servers":[{"url":"https://node.invalid"}],"paths":{"/echo":{"post":{"requestBody":{"content":{"application/json":{"schema":true}}},"responses":{"200":{"description":"echo","content":{"application/json":{"schema":true}}}}}}}}';
const input = '{"body":{"n":9007199254740993,"label":"雪"}}';
const output = '{"n":9007199254740993,"label":"雪"}';
const cases = [
  {kind: 'local.openapi-json-probe.r1', description},
  {kind: 'local.openapi30-json-probe.r1', description: '{"openapi":"3.0.4","info":{"title":"Node 3.0","version":"1"},"servers":[{"url":"https://node.invalid"}],"paths":{"/echo":{"post":{"requestBody":{"content":{"application/json":{"schema":{"type":"object","nullable":true}}}},"responses":{"200":{"description":"echo","content":{"application/json":{"schema":{"type":"object","nullable":true}}}}}}}}}'},
];
async function cycle() {
 for (const {kind, description} of cases) {
  if (await exact_roundtrip(kind, description, input, output) !== output) throw new Error('exact output changed');
  let failed = false;
  try { await exact_roundtrip(kind, description, input, '{'); } catch { failed = true; }
  if (!failed) throw new Error('malformed success accepted');
  let challenged = false;
  const choices = description.replace('{"url":"https://node.invalid"}', '{"url":"https://node.invalid"},{"url":"https://second.invalid"}');
  try { await exact_roundtrip(kind, choices, input, output); } catch (e) { challenged = String(e) === "CONTEXT_REQUIRED"; }
  if (!challenged) throw new Error("context challenge lost");
  await pending_cleanup(kind, description, false);
  await pending_cleanup(kind, description, true);
 }
}
async function main() {
  const crudColdOwners = logical_owners();
  await crud_consumers();
  const crudWarmOwners = logical_owners();
  await crud_consumers();
  if (logical_owners() !== crudWarmOwners) throw new Error(`CRUD retained owners ${crudWarmOwners} -> ${logical_owners()}`);
  console.log(JSON.stringify({crudExchangesPerRun:48, crudRuns:2, crudColdOwners, crudWarmOwners, crudFinalOwners:logical_owners()}));
  for (const v30 of [false,true]) {
    const kind=v30 ? 'local.openapi30-json-probe.r1' : 'local.openapi-json-probe.r1';
    for (const [name,path,method,input,header] of [['ServiceToken','/items','get','{}','authorization'],['PartnerKey','/items/new','post','{"body":{"n":9007199254740993,"label":"雪"}}','x-partner-key']]) {
      const context=JSON.stringify({credentials:{[name]:'node-token-opaque-A'},configuration:{[kind+'.security']:{schemes:[name],origins:{[name]:['https://api.example.test']}}}});
      const receipt=JSON.parse(await authenticated_call(v30,path,method,context,input));
      if(receipt.code!==null || receipt.hostEntries!==1 || receipt.output[0]!=='{"ok":true,"n":9007199254740993,"label":"雪"}') throw new Error('JS context success contract');
      if(receipt.wires[0].body !== (method==='post'?'{"n":9007199254740993,"label":"雪"}':null)) throw new Error('JS request JSON text changed');
      const values=receipt.wires[0].headers.filter((h:string[])=>h[0]===header);
      if(values.length!==1 || values[0][1] !== (name==='ServiceToken'?'Bearer ':'')+'node-token-opaque-A') throw new Error('JS credential changed');
      for(const [ctx,code] of [['{}','CONTEXT_REQUIRED'],[context.replace('https://api.example.test','https://wrong.test'),'ERR_REFUSED'],[context.replace('node-token-opaque-A','bad\\r\\nvalue'),'ERR_REFUSED']]){
        const failure=JSON.parse(await authenticated_call(v30,path,method,ctx,input));
        if(failure.code!==code || failure.hostEntries!==0 || failure.output.length!==0) throw new Error('JS context failure effects');
      }
    }
  }
  await authenticated_consumers();
  await cycle();
  const before = logical_owners();
  for (let i=0;i<1000;i++) await cycle();
  if (logical_owners() !== before) throw new Error(`retained owners ${before} -> ${logical_owners()}`);
  console.log(JSON.stringify({cyclesPerKind:1000, kinds:cases.map(c=>c.kind), exactText:true, localPendingHost:true, owners:before}));
}
void main().catch(error => { console.error(error); throw error; });
