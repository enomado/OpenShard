// An opt-in static-atlas exhaustion path for openshard-playground.
//
// The server moves the same player the client is rendering, rather than
// panning a second camera or teleporting between samples. That leaves the
// normal movement packets, player-follow camera and map-static collection in
// the measurement. At the world's 20Hz tick, the route has crossed more than
// 7,000 map tiles after about six minutes; its expanding 96-tile square is wide
// enough to introduce substantially more static art than one 2048px atlas
// holds on the stock Felucca Britain map.
//
// A configured client install can have a local wall or a patch-dependent
// impassable cell on one of those legs. `StepRefused` therefore rotates the
// route clockwise immediately. This avoids spending the rest of a leg against
// one obstacle while retaining a deterministic path for a given map.

const DIRECTIONS = [2, 4, 6, 0]; // east, south, west, north
const FIRST_LEG = 96;
const LEG_GROWTH = 96;

let player = 0;
let direction = 0;
let remaining = FIRST_LEG;
let leg = FIRST_LEG;
let turns = 0;

function turn() {
    direction = (direction + 1) % DIRECTIONS.length;
    turns += 1;
    if (turns % 2 === 0) {
        leg += LEG_GROWTH;
    }
    remaining = leg;
}

function onEvent(event) {
    if (event.type === "PlayerEntered") {
        player = event.serial;
        Deno.core.ops.op_control(player);
    }
    if (event.type === "StepRefused" && event.serial === player) {
        turn();
    }
}

function onTick(serial) {
    if (serial !== player) {
        return;
    }
    Deno.core.ops.op_move(serial, DIRECTIONS[direction]);
    remaining -= 1;
    if (remaining === 0) {
        turn();
    }
}
