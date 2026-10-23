"""Throwaway Modbus TCP slave for validate-rustred.sh: FC1-4 over fixed values (see tests/fixtures.ts).

    python3 modbus_sim.py PORT [big|little|swapped]
"""
import socket, struct, threading, sys
PORT=int(sys.argv[1]); ORDER=sys.argv[2] if len(sys.argv)>2 else "big"
def words(fmt, v, n):
    b = struct.pack(">"+fmt, v)
    w = [struct.unpack(">H", b[i:i+2])[0] for i in range(0, n*2, 2)]
    if ORDER=="little": w=w[::-1]
    if ORDER=="swapped": w=[((x&0xff)<<8)|(x>>8) for x in w]
    return w
hold={}; inp={}; coil={0:1}; disc={5:1}
for i,w in enumerate(words("f",230.5,2)): hold[3000+i]=w
for i,w in enumerate(words("f",231.25,2)): hold[3002+i]=w
for i,w in enumerate(words('H',42,1)): hold[10+i]=w
for i,w in enumerate(words('h',215,1)): hold[20+i]=w
for i,w in enumerate(words("i",-70000,2)): inp[100+i]=w
for i,w in enumerate(words("d",3.141592653589793,4)): inp[200+i]=w
def handle(c):
    while True:
        h=c.recv(7)
        if len(h)<7: return
        tid,pid,ln,uid=struct.unpack(">HHHB",h)
        pdu=c.recv(ln-1)
        fc=pdu[0]; addr,qty=struct.unpack(">HH",pdu[1:5])
        if fc in(3,4):
            m=hold if fc==3 else inp
            vals=[m.get(addr+i,0) for i in range(qty)]
            body=bytes([fc,qty*2])+b"".join(struct.pack(">H",v) for v in vals)
        elif fc in(1,2):
            m=coil if fc==1 else disc
            bits=[m.get(addr+i,0) for i in range(qty)]
            nb=(qty+7)//8; arr=bytearray(nb)
            for i,b in enumerate(bits):
                if b: arr[i//8]|=1<<(i%8)
            body=bytes([fc,nb])+bytes(arr)
        else: body=bytes([fc|0x80,1])
        c.sendall(struct.pack(">HHHB",tid,0,len(body)+1,uid)+body)
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); s.bind(("127.0.0.1",PORT)); s.listen(5)
while True:
    c,_=s.accept(); threading.Thread(target=handle,args=(c,),daemon=True).start()
