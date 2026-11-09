"""Explicit deployment helper for the authorized test VPS; no compilation.

Rust generates and publishes all artifacts. This driver backs up the deployment,
activates the non-root companion, and validates the unchanged subscription URLs.
"""
import argparse, base64, hashlib, http.client, json, os, re, secrets, shutil, socket, ssl, subprocess, time, uuid
from pathlib import Path

os.umask(0o077)
p=argparse.ArgumentParser()
p.add_argument('--candidate',type=Path,required=True)
p.add_argument('--sha256',required=True)
p.add_argument('--evidence',type=Path,required=True)
a=p.parse_args()
stage=a.evidence.resolve()
assert stage.parent==Path('/root') and stage.name.startswith('sbctl-subscription-publish-')
stage.mkdir(mode=0o700,exist_ok=False)
live=Path('/usr/local/bin/sbctl')
source=Path('/etc/sbctl/protocol-presets.json')
unit=Path('/etc/systemd/system/sbctl-xray.service')
directory=Path('/etc/sbctl-xray')
xray=Path('/usr/local/bin/sbctl-xray')
artifacts=Path('/var/lib/sbctl/artifacts')
config=Path('/etc/sbctl/config.toml')
report={}

def run(label,cmd,timeout=90):
    r=subprocess.run([str(x) for x in cmd],capture_output=True,text=True,timeout=timeout)
    (stage/(label+'.log')).write_text(r.stdout+r.stderr)
    if r.returncode: raise RuntimeError(label+' failed; private log retained')
    return r.stdout

def get(route):
    h=http.client.HTTPSConnection(host,timeout=20,context=ssl.create_default_context())
    h._create_connection=lambda *args,**kw:socket.create_connection(('127.0.0.1',443),20)
    h.request('GET','/sub/'+credential+'/'+route)
    r=h.getresponse();body=r.read();status=r.status;h.close()
    assert status==200 and body, 'published subscription unavailable'
    return body

def atomic_copy(src,dst,mode=0o755):
    tmp=dst.with_name(dst.name+'.preset-new');shutil.copyfile(src,tmp);tmp.chmod(mode);os.replace(tmp,dst)

paths=[live,source,unit,xray,directory/'config.json']
before={str(path):path.exists() for path in paths}
try:
    assert hashlib.sha256(a.candidate.read_bytes()).hexdigest()==a.sha256,'candidate digest mismatch'
    assert not source.exists() and not unit.exists(),'additional deployment already exists; use its explicit update path'
    text=config.read_text()
    host=re.search(r'^subscription_host = "([^"]+)"',text,re.M).group(1)
    credential=re.search(r'^subscription_credential = "([^"]+)"',text,re.M).group(1)
    identity=hashlib.sha256((host+'\n'+credential).encode()).hexdigest()
    old_uri=get('uri').decode().splitlines()
    for i,path in enumerate(paths):
        if path.exists(): shutil.copy2(path,stage/('before-'+str(i)))
    shutil.copytree(artifacts,stage/'before-artifacts')
    pinned=Path('/var/lib/sbctl/certificates')/host
    cert=pinned/'fullchain.pem';key=pinned/'privkey.pem'
    assert cert.is_file() and key.is_file(),'managed domain certificate unavailable'
    core=Path('/opt/sbctl-protocol-test-18c79cb/xray-linux-amd64')
    keypair=run('generate-reality-key',[core,'x25519'])
    private=re.search(r'(?:PrivateKey|Private key):\s*(\S+)',keypair).group(1)
    public=re.search(r'(?:Password(?: \(PublicKey\))?|PublicKey|Public key):\s*(\S+)',keypair).group(1)
    nodes=[];reserved=[]
    for name,preset in [('VLESS XHTTP Reality','vless-xhttp-reality'),('VLESS XHTTP TLS','vless-xhttp-tls'),('VLESS WS TLS','vless-ws-tls'),('Trojan TCP TLS','trojan-tcp-tls')]:
        while True:
            tcp=socket.socket();tcp.bind(('0.0.0.0',0));port=tcp.getsockname()[1]
            udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
            try: udp.bind(('0.0.0.0',port))
            except OSError: tcp.close();udp.close();continue
            if port>=10000: break
            tcp.close();udp.close()
        reserved.extend([tcp,udp])
        node={'name':name,'preset':preset,'server':host,'server_name':host,'port':port,'credential':secrets.token_urlsafe(32) if preset.startswith('trojan') else str(uuid.uuid4())}
        if not preset.startswith('trojan'):node['path']='/'+secrets.token_urlsafe(18)
        if preset.endswith('reality'):
            node['server_name']='www.microsoft.com'
            node['reality']={'private_key':private,'public_key':public,'short_id':secrets.token_hex(8),'target':'www.microsoft.com:443'}
        nodes.append(node)
    input_path=stage/'nodes.json';input_path.write_text(json.dumps(nodes))
    candidate_config=run('render-xray',[a.candidate,'protocol','export','--input',input_path,'--format','xray-server','--certificate',cert,'--private-key',key])
    check=stage/'xray-server.json';check.write_text(candidate_config)
    run('check-xray',[core,'run','-test','-c',check])
    atomic_copy(core,xray)
    import grp
    group=grp.getgrnam('sing-box').gr_gid
    directory.mkdir(mode=0o750,exist_ok=True);os.chown(directory,0,group);directory.chmod(0o750)
    atomic_copy(check,directory/'config.json',0o640);os.chown(directory/'config.json',0,group)
    unit.write_text('[Unit]\nDescription=sbctl managed protocol companion\nAfter=network-online.target sing-box.service\nRequires=sing-box.service\nPartOf=sing-box.service\n\n[Service]\nType=simple\nUser=sing-box\nGroup=sing-box\nExecStart=/usr/local/bin/sbctl-xray run -c /etc/sbctl-xray/config.json\nRestart=on-failure\nRestartSec=2\nNoNewPrivileges=true\nPrivateTmp=true\nProtectHome=true\nProtectSystem=strict\n\n[Install]\nWantedBy=multi-user.target\n')
    unit.chmod(0o644)
    for s in reserved:s.close()
    run('reload',['systemctl','daemon-reload'])
    run('start-companion',['systemctl','enable','--now','sbctl-xray.service'])
    time.sleep(2)
    run('companion-active',['systemctl','is-active','--quiet','sbctl-xray.service'])
    atomic_copy(a.candidate,live)
    run('publish',[live,'protocol','enable','--input',input_path])
    run('restart-sbctl',['systemctl','restart','sbctl.service'])
    time.sleep(1)
    new_uri=get('uri').decode().splitlines()
    assert set(old_uri).issubset(new_uri),'existing proxy links changed'
    assert len(new_uri)==len(old_uri)+4,'additional URI nodes missing'
    page=get('index').decode()
    assert all(n['name'] in page for n in nodes),'overview missing additional nodes'
    for route in ['sing-box.json','sing-box-full.json','sing-box-1.10.json','sing-box-1.11.json','sing-box-1.12.json','sing-box-1.13.json','sing-box-1.14.json']:
        parsed=json.loads(get(route));tags={entry['tag'] for entry in parsed['outbounds']}
        assert nodes[2]['name'] in tags and nodes[3]['name'] in tags
        assert nodes[0]['name'] not in tags and nodes[1]['name'] not in tags
    current=get('clash.yaml').decode();legacy=get('clash-1.18.yaml').decode()
    assert all(n['name'] in current for n in nodes[1:]) and nodes[0]['name'] not in current
    assert all(n['name'] in legacy for n in nodes[2:]) and all(n['name'] not in legacy for n in nodes[:2])
    shadow=base64.b64decode(get('shadowrocket.txt')).decode()
    assert all(n['name'].replace(' ','%20') in shadow for n in nodes[2:])
    assert config.read_text()==text,'deployment identity/configuration changed'
    assert base64.b64decode(get('uri.txt')).decode().splitlines()==new_uri
    run('check-published-singbox',['/usr/local/bin/sing-box','check','-c',artifacts/'subscription-sing-box-full.json'])
    run('check-published-mihomo',['/opt/sbctl-protocol-test-18c79cb/mihomo','-t','-d',stage,'-f',artifacts/'subscription-clash.yaml'])
    client_exports=json.loads(run('render-clients',[live,'protocol','export','--input',input_path,'--format','xray-client']))
    connectivity={}
    for i,node in enumerate(nodes):
        sock=socket.socket();sock.bind(('127.0.0.1',0));local_port=sock.getsockname()[1];sock.close()
        client={'inbounds':[{'listen':'127.0.0.1','port':local_port,'protocol':'socks','settings':{'auth':'noauth'}}], 'outbounds':[client_exports['outbounds'][i]]}
        client_path=stage/('client-'+str(i)+'.json');client_path.write_text(json.dumps(client))
        with (stage/('client-'+str(i)+'.log')).open('w') as log:
            child=subprocess.Popen([str(core),'run','-c',str(client_path)],stdout=log,stderr=log)
            try:
                time.sleep(1)
                assert child.poll() is None,'client failed to start'
                run('connect-'+str(i),['curl','--silent','--show-error','--fail','--max-time','40','--noproxy','','--socks5-hostname','127.0.0.1:'+str(local_port),'https://example.com/'])
                connectivity[node['name']]='verified TLS and HTTPS egress'
            finally:
                child.terminate();child.wait(timeout=5)
    report={'passed':True,'subscription_identity_sha256':identity,'unchanged_existing_nodes':len(old_uri),'added_nodes':4,'uri_nodes':len(new_uri),'connectivity':connectivity,'ports':{n['name']:n['port'] for n in nodes},'services':{n:run('state-'+n,['systemctl','is-active',n]).strip() for n in ['sbctl','sing-box','sbctl-xray']}}
except Exception as e:
    if unit.exists():subprocess.run(['systemctl','disable','--now','sbctl-xray.service'],capture_output=True)
    for i,path in enumerate(paths):
        backup=stage/('before-'+str(i))
        if backup.exists():atomic_copy(backup,path,backup.stat().st_mode&0o777)
        elif not before[str(path)] and path.exists():path.unlink()
    if (stage/'before-artifacts').is_dir():
        for path in artifacts.glob('subscription-*'):
            if path.is_file() and not (stage/'before-artifacts'/path.name).exists():path.unlink()
        for path in (stage/'before-artifacts').iterdir():
            if path.is_file():shutil.copy2(path,artifacts/path.name)
    subprocess.run(['systemctl','daemon-reload'],capture_output=True)
    subprocess.run(['systemctl','restart','sbctl.service'],capture_output=True)
    subprocess.run([str(live),'regenerate'],capture_output=True)
    report={'passed':False,'error':str(e),'rollback':True}
finally:
    (stage/'result.json').write_text(json.dumps(report,indent=2));print(json.dumps(report),flush=True)
raise SystemExit(0 if report.get('passed') else 1)
