import json, random, time, urllib.request
random.seed(20260717)
CLOUD="http://127.0.0.1:8080/v1/ingest"; BEARER="devkey"; ORG="meridian.example"
now=int(time.time()*1000); DAY=8*3600*1000; start=now-DAY
# enterprise per-call cost (microusd): haiku cheap high-volume; sonnet mid+tools; gpt-4o big-context/reasoning
FLEET=[
 ("fraud","fraud-triage-copilot","claude-sonnet-5",120,(3,7),(45000,150000),0.10),
 ("fraud","txn-anomaly-scorer","claude-haiku-4-5",700,(1,3),(2000,11000),0.28),
 ("kyc-aml","kyc-intake-agent","claude-sonnet-5",160,(2,5),(38000,120000),0.08),
 ("kyc-aml","sanctions-screener","claude-haiku-4-5",560,(1,2),(1500,7000),0.35),
 ("kyc-aml","aml-case-copilot","gpt-4o",90,(4,9),(180000,720000),0.05),
 ("lending","underwriting-copilot","claude-sonnet-5",140,(5,11),(60000,210000),0.06),
 ("lending","doc-intake-ocr","claude-haiku-4-5",480,(1,3),(3000,15000),0.22),
 ("lending","collateral-valuator","gpt-4o-mini",210,(2,4),(9000,45000),0.12),
 ("support","support-tier1-bot","claude-haiku-4-5",980,(1,2),(1200,6000),0.40),
 ("support","support-tier2-bot","claude-sonnet-5",280,(2,4),(30000,95000),0.18),
 ("support","escalation-router","claude-haiku-4-5",340,(1,1),(900,3500),0.30),
 ("treasury","cashflow-forecaster","gpt-4o",60,(6,12),(220000,820000),0.04),
 ("treasury","spend-optimizer","claude-sonnet-5",80,(3,6),(40000,130000),0.10),
 ("compliance","control-tester","claude-haiku-4-5",190,(2,4),(2500,12000),0.15),
 ("compliance","evidence-assembler","claude-sonnet-5",55,(4,8),(48000,160000),0.06),
]
recs=[]
def add(run,ag,model,dec,cin,cout,cost,step,ts): recs.append({"ts_millis":ts,"run_id":run,"model":model,"decision":dec,"input_tokens":cin,"output_tokens":cout,"cost_microusd":cost,"step":step,"agent_id":ag})
for team,name,model,nr,cpr,crng,cache in FLEET:
    ag=f"agent://{ORG}/{team}/{name}"
    for r in range(nr):
        run=f"{name}-{r:04d}"; nc=random.randint(*cpr); ts0=start+random.randint(0,DAY-1)
        for s in range(nc):
            ts=min(now-1, ts0+s*random.randint(200,4000)); cin=random.randint(600,12000); cout=random.randint(150,3000)
            if random.random()<cache: add(run,ag,model,"cache_hit",cin,cout,0,s,ts)
            else: add(run,ag,model,"allow",cin,cout,random.randint(*crng),s,ts)
sup=f"agent://{ORG}/support/support-tier1-bot"
for i in range(9): add(f"support-tier1-bot-dlp-{i:02d}",sup,"claude-haiku-4-5","dlp_blocked",1400,320,4000,0,start+random.randint(0,DAY))
uw=f"agent://{ORG}/lending/underwriting-copilot"
for i in range(6): add(f"underwriting-copilot-pol-{i:02d}",uw,"claude-sonnet-5","policy_violation",2100,240,85000,0,start+random.randint(0,DAY))
rag=f"agent://{ORG}/treasury/reconciliation-batch"; rr="reconciliation-batch-eod-001"; tsb=now-45*60*1000
for s in range(16): add(rr,rag,"gpt-4o","allow",8000+s*1500,1500+s*300,120000+s*30000,s,tsb+s*18000)
for s in range(16,58): add(rr,rag,"gpt-4o","budget_exceeded",12000,3000,62000,s,tsb+s*18000)  # 42 blocks ~ $2.6k prevented
def post(r):
    b=json.dumps({"records":r}).encode()
    urllib.request.urlopen(urllib.request.Request(CLOUD,data=b,method="POST",headers={"Authorization":f"Bearer {BEARER}","Content-Type":"application/json"}),timeout=60).read()
random.shuffle(recs); tot=0
for i in range(0,len(recs),1000): post(recs[i:i+1000]); tot+=len(recs[i:i+1000])
print(f"ingested {tot} records, {len(FLEET)+1} agents")
