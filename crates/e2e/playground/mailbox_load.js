// An opt-in mailbox exercise for openshard-playground.
//
// One player entry creates a small crowd near that player. Every creature is
// then driven through the ordinary world tick, producing the same incoming and
// movement packets a populated shard does. It deliberately does not use the
// client's resync path: replaying full snapshots measures renderer churn, not
// sustained live traffic.

function onEvent(event) {
    if (event.type === "PlayerEntered") {
        for (let row = 0; row < 8; row += 1) {
            for (let column = 0; column < 8; column += 1) {
                Deno.core.ops.op_spawn_mobile({
                    body: 0x0190,
                    hits: 50,
                    x: event.x + column + 2,
                    y: event.y + row + 2,
                    z: event.z,
                    name: "mailbox walker",
                });
            }
        }
    }
    if (event.type === "MobileSpawned") {
        Deno.core.ops.op_control(event.serial);
    }
}

function onTick(serial) {
    // East/west alternation gives the walker room to move without teaching the
    // diagnostic scenario gameplay or an artificial packet encoder.
    Deno.core.ops.op_move(serial, serial & 1 ? 2 : 6);
}
