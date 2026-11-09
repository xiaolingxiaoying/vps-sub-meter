"""Exercise an explicitly supplied Xray outbound list from the build host.

Contains no server credentials; input/output logs belong in private scratch.
"""
import argparse,json,socket,subprocess,time
from pathlib import Path

p=argparse.ArgumentParser()
p.add_argument('--input',type=Path,required=True)
p.add_argument('--xray',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
a=p.parse_args()
nodes=json.loads(a.input.read_text())['outbounds']
report=[]
for i,outbound in enumerate(nodes):
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    path=a.input.parent/('probe-'+str(i)+'.json')
    path.write_text(json.dumps({'inbounds':[{'listen':'127.0.0.1','port':port,'protocol':'socks','settings':{'auth':'noauth'}}],'outbounds':[outbound]}))
    with (a.input.parent/('probe-'+str(i)+'.log')).open('w') as log:
        child=subprocess.Popen([str(a.xray),'run','-c',str(path)],stdout=log,stderr=log)
        try:
            time.sleep(1)
            assert child.poll() is None, 'test client failed to start'
            response=subprocess.run(['curl','--silent','--show-error','--fail','--max-time','45','--noproxy','','--socks5-hostname','127.0.0.1:'+str(port),'https://example.com/'],capture_output=True,timeout=50)
            report.append({'node':outbound['tag'],'passed':response.returncode==0 and bool(response.stdout),'bytes':len(response.stdout),'curl_exit':response.returncode})
        finally:
            child.terminate();child.wait(timeout=5)
a.output.write_text(json.dumps(report,indent=2))
print(json.dumps(report))
raise SystemExit(0 if all(row['passed'] for row in report) else 1)
