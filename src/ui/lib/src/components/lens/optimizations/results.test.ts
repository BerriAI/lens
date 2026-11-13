import { describe, expect, it } from "vitest";
import { defaultConfig, type Optimization } from "./model";
import { measuredRows } from "./results";

function run(mode: "live" | "offline" = "offline"): Optimization {
  return {
    id: "experiment", config: {...defaultConfig(mode), baseline:"baseline", models:["candidate"], judge:"llm"},
    status:"completed", createdAt:0, seed:false, completed:60, target:60, examples:[],
    measured: Array.from({length:60},(_,index)=>({
      id:String(index),sourceId:String(index%30),model:index<30?"baseline":"candidate",servedModel:null,
      traceId:"trace",spanId:"span",input:"question",expected:"answer",original:"answer",output:"answer",score:8,
      costUsd:index<30?0.02:0.01,durationMs:1000,error:null,scoreError:null,completedAt:0,
    })),
  };
}

describe("measured model comparisons",()=>{
  it("recommends cheaper paired results only after enough scored evidence",()=>{
    const comparison=run();
    expect(measuredRows(comparison)[1]).toMatchObject({qualifies:true,savings:50,scored:30});
    expect(measuredRows({...comparison,measured:comparison.measured?.slice(0,59)})[1].qualifies).toBe(false);
  });
  it("keeps missing cost unknown and suppresses savings recommendations",()=>{
    const comparison=run();
    comparison.measured![30].costUsd=null;
    expect(measuredRows(comparison)[1].cost).toBeNaN();
    expect(measuredRows(comparison)[1].qualifies).toBe(false);
  });
  it("does not hide baseline errors when assessing candidate quality",()=>{
    const comparison=run();
    comparison.measured![0].scoreError="Judge unavailable";
    expect(measuredRows(comparison)[1].qualifies).toBe(false);
  });
  it("requires comparable feedback coverage in live traffic",()=>{
    const comparison=run("live"); comparison.config.judge="feedback";
    comparison.measured=comparison.measured!.map((result,index)=>({...result,score:index>40?null:8}));
    expect(measuredRows(comparison)[1].qualifies).toBe(false);
  });
  it("rejects a cheaper candidate with worse quality",()=>{
    const comparison=run();
    comparison.measured=comparison.measured!.map(result=>({...result,score:result.model==="candidate"?6:8}));
    expect(measuredRows(comparison)[1].qualifies).toBe(false);
  });
});
