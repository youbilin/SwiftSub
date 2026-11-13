package main
import("crypto/tls";"net/http";"os")
func main(){s:=&http.Server{Addr:os.Args[1],Handler:http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){w.Write([]byte("target"))}),TLSConfig:&tls.Config{MinVersion:tls.VersionTLS13}};panic(s.ListenAndServeTLS(os.Args[2],os.Args[3]))}
