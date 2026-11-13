"""Exercise generated links through caller-supplied Xray and Mihomo on loopback.
Build: scripts/cargo-msvc.bat build -p swiftsub-core --example validation_profile
No software is downloaded. All listeners are loopback and processes are cleaned up.
"""
import argparse, http.server, json, os, pathlib, re, socket, subprocess, tempfile, threading, time, uuid

def port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]

def ready(number, process=None):
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        if process is not None and process.poll() is not None: raise RuntimeError("process exited during startup")
        try:
            with socket.create_connection(("127.0.0.1",number),timeout=.2): return
        except OSError: time.sleep(.1)
    raise RuntimeError("listener did not become ready")

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body=b"swiftsub-controlled-probe"
        self.send_response(200);self.send_header("Content-Length",str(len(body)));self.end_headers();self.wfile.write(body)
    def log_message(self,*args): pass

def main():
    p=argparse.ArgumentParser();p.add_argument("--xray",required=True);p.add_argument("--mihomo",required=True)
    p.add_argument("--generator",default="target/debug/examples/validation_profile.exe")
    p.add_argument("--data-dir",required=True,help="Mihomo data directory containing GeoSite.dat")
    p.add_argument("--openssl",default="openssl");p.add_argument("--tls-target",help="optional Go TLS test server built from scripts/fixtures/tls-target.go");a=p.parse_args()
    root=pathlib.Path(tempfile.mkdtemp(prefix="swiftsub-loopback-"));results=[];processes=[];servers=[];logs=[]
    flags=subprocess.CREATE_NO_WINDOW if os.name=="nt" else 0
    def launch(argv,name):
        f=open(root/(name+".log"),"w",encoding="utf-8");logs.append(f)
        child=subprocess.Popen(argv,stdout=f,stderr=subprocess.STDOUT,creationflags=flags);processes.append(child);return child
    try:
        dest=port();tls=port()
        server=http.server.ThreadingHTTPServer(("127.0.0.1",dest),Handler)
        servers.append(server);threading.Thread(target=server.serve_forever,daemon=True).start()
        (root/"openssl.cnf").write_text("[req]\ndistinguished_name=dn\n[dn]\n",encoding="utf-8")
        subprocess.run([a.openssl,"req","-config",str(root/"openssl.cnf"),"-x509","-newkey","rsa:2048","-nodes","-keyout",str(root/"key.pem"),"-out",str(root/"cert.pem"),"-days","1","-subj","/CN=localhost"],check=True,capture_output=True,creationflags=flags)
        tls_command=([a.tls_target,f"127.0.0.1:{tls}",str(root/"cert.pem"),str(root/"key.pem")] if a.tls_target else [a.openssl,"s_server","-accept",f"127.0.0.1:{tls}","-cert",str(root/"cert.pem"),"-key",str(root/"key.pem"),"-tls1_3","-www"])
        tp=launch(tls_command,"tls-target")
        ready(tls,tp)
        keys=subprocess.check_output([a.xray,"x25519"],text=True,creationflags=flags)
        private=re.search(r"PrivateKey:\s*(\S+)",keys).group(1)
        public=re.search(r"(?:Password \(PublicKey\)|PublicKey):\s*(\S+)",keys).group(1)
        identity=str(uuid.uuid4())
        for name,network,reality,mlkem in [("xhttp","xhttp",False,True),("reality-default","raw",True,None),("reality-mlkem-on","raw",True,True),("reality-mlkem-off","raw",True,False),("xhttp-reality","xhttp",True,True)]:
            upstream=port();mixed=port()
            stream={"network":network,"security":"reality" if reality else "none"}
            if network=="xhttp":stream["xhttpSettings"]={"path":"/probe","mode":"packet-up"}
            if reality:stream["realitySettings"]={"target":f"127.0.0.1:{tls}","serverNames":["localhost"],"show":True,"privateKey":private,"shortIds":["1234"]}
            config={"log":{"loglevel":"info"},"inbounds":[{"listen":"127.0.0.1","port":upstream,"protocol":"vless","settings":{"clients":[{"id":identity}],"decryption":"none"},"streamSettings":stream}],"outbounds":[{"protocol":"freedom","settings":{"finalRules":[{"action":"allow","ip":["127.0.0.1/32"]},{"action":"block"}]}}]}
            config_path=root/(name+"-server.json");config_path.write_text(json.dumps(config),encoding="utf-8")
            xp=launch([a.xray,"run","-config",str(config_path)],name+"-xray")
            try:
                ready(upstream,xp)
                query=f"type={network}"
                if network=="xhttp":query+="&path=%2Fprobe&mode=packet-up"
                if reality:
                    query+=f"&security=reality&pbk={public}&sid=1234&sni=localhost&fp=chrome"
                    if mlkem is not None:query+=f"&support-x25519mlkem768={str(mlkem).lower()}"
                link=root/(name+".txt");link.write_text(f"vless://{identity}@127.0.0.1:{upstream}?{query}#controlled-{name}",encoding="utf-8")
                profile=root/(name+".yaml")
                subprocess.run([a.generator,str(link),str(profile),str(mixed)],check=True,capture_output=True,creationflags=flags)
                mp=launch([a.mihomo,"-d",a.data_dir,"-f",str(profile)],name+"-mihomo")
                ready(mixed,mp)
                r=subprocess.run(["curl.exe" if os.name=="nt" else "curl","--fail","--silent","--show-error","--max-time","15","--noproxy","","--proxy",f"http://127.0.0.1:{mixed}",f"http://127.0.0.1:{dest}/probe"],capture_output=True,text=True,creationflags=flags)
                passed=r.returncode==0 and r.stdout=="swiftsub-controlled-probe"
                results.append({"case":name,"passed":passed,"curl_exit":r.returncode,"detail":r.stderr.strip()})
                print(json.dumps(results[-1]),flush=True)
                mp.terminate();mp.wait(timeout=5)
            except Exception as error:results.append({"case":name,"passed":False,"detail":str(error)})
            finally:
                if xp.poll() is None:xp.terminate();xp.wait(timeout=5)
    finally:
        for child in processes:
            if child.poll() is None:
                child.terminate()
                try:child.wait(timeout=5)
                except subprocess.TimeoutExpired:child.kill();child.wait()
        for server in servers:server.shutdown();server.server_close()
        for f in logs:f.close()
        (root/"results.json").write_text(json.dumps(results,indent=2),encoding="utf-8")
        print("Evidence directory:",root,flush=True)
    return 0 if len(results)==5 and all(r["passed"] for r in results) else 1

if __name__=="__main__":raise SystemExit(main())
