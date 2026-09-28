// Minimal loopback-only STUN Binding fixture, matching run.py's native fixture.
import {createSocket} from 'node:dgram';
export async function startLoopbackStun() {
  const socket = createSocket('udp4');
  socket.on('message', (message, remote) => {
    if (message.length < 20 || message.readUInt16BE(0) !== 1 || message.readUInt32BE(4) !== 0x2112a442) return;
    const reply = Buffer.alloc(32);
    reply.writeUInt16BE(0x101, 0); reply.writeUInt16BE(12, 2); reply.writeUInt32BE(0x2112a442, 4);
    message.copy(reply, 8, 8, 20);
    reply.writeUInt16BE(0x20, 20); reply.writeUInt16BE(8, 22); reply[25] = 1;
    reply.writeUInt16BE(remote.port ^ 0x2112, 26);
    const address = remote.address.split('.').reduce((value, octet) => (value * 256 + Number(octet)) >>> 0, 0);
    reply.writeUInt32BE((address ^ 0x2112a442) >>> 0, 28);
    socket.send(reply, remote.port, remote.address);
  });
  await new Promise((resolve, reject) => { socket.once('error', reject); socket.bind(0, '127.0.0.1', resolve); });
  return {url: `stun:127.0.0.1:${socket.address().port}`, close: () => new Promise(resolve => socket.close(resolve))};
}
