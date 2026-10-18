/* Pyraminx core -- edge-only model.
   12-element state: edge[i] = which edge piece is at position i.
   Edge positions (0-5):
     0: U/L  between U and L faces
     1: U/R  between U and R faces
     2: U/B  between U and B faces
     3: L/R  between L and R faces
     4: R/B  between R and B faces
     5: L/B  between L and B faces
   Solved: edge[i] = i (each position holds its own edge piece).
   Edge piece i carries two face-colors: [face_A, face_B].
   Solved colors: edge 0=[0,1], 1=[0,2], 2=[0,3], 3=[1,2], 4=[2,3], 5=[1,3].
   Edge cycles per face turn:
     U: edges 0->1->2->0 (U-L, U-R, U-B cycle)
     L: edges 0->3->5->0 (U-L, L/R, L/B cycle)
     R: edges 1->3->4->1 (U-R, L/R, R/B cycle)
     B: edges 2->4->5->2 (U-B, R/B, L/B cycle)
*/
"use strict";
(function () {

  var SOLVED = [0, 1, 2, 3, 4, 5];

  function applyMove(state, tok) {
    var s = state.slice();
    if (tok === "U") {
      var t0 = s[0], t1 = s[1], t2 = s[2];
      s[0] = t1; s[1] = t2; s[2] = t0;
    } else if (tok === "U'") {
      var t0 = s[0], t1 = s[1], t2 = s[2];
      s[0] = t2; s[1] = t0; s[2] = t1;
    } else if (tok === "U2") {
      var t0 = s[0], t2 = s[2];
      s[0] = t2; s[2] = t0;
    } else if (tok === "L") {
      var t0 = s[0], t3 = s[3], t5 = s[5];
      s[0] = t3; s[3] = t5; s[5] = t0;
    } else if (tok === "L'") {
      var t0 = s[0], t3 = s[3], t5 = s[5];
      s[0] = t5; s[3] = t0; s[5] = t3;
    } else if (tok === "L2") {
      var t0 = s[0], t3 = s[3];
      s[0] = t3; s[3] = t0;
    } else if (tok === "R") {
      var t1 = s[1], t3 = s[3], t4 = s[4];
      s[1] = t3; s[3] = t4; s[4] = t1;
    } else if (tok === "R'") {
      var t1 = s[1], t3 = s[3], t4 = s[4];
      s[1] = t4; s[3] = t1; s[4] = t3;
    } else if (tok === "R2") {
      var t1 = s[1], t3 = s[3];
      s[1] = t3; s[3] = t1;
    } else if (tok === "B") {
      var t2 = s[2], t4 = s[4], t5 = s[5];
      s[2] = t4; s[4] = t5; s[5] = t2;
    } else if (tok === "B'") {
      var t2 = s[2], t4 = s[4], t5 = s[5];
      s[2] = t5; s[4] = t2; s[5] = t4;
    } else if (tok === "B2") {
      var t2 = s[2], t4 = s[4];
      s[2] = t4; s[4] = t2;
    }
    return s;
  }

  function scramble(seed, length) {
    var rng = (seed >>> 0) || 1;
    var rnd = function () {
      rng = (Math.imul(rng, 1664525) + 1013904223) | 0;
      return (rng >>> 0) / 0x100000000;
    };
    var faces = ["U", "L", "R", "B"];
    var suffs = ["", "'", "2"];
    var moves = [];
    var last = "";
    var i;
    for (i = 0; i < (length || 20); i++) {
      var f;
      do { f = faces[(rnd() * 4) | 0]; } while (f === last);
      last = f;
      moves.push(f + suffs[(rnd() * 3) | 0]);
    }
    return moves;
  }

  function verifySolved(state) {
    var i;
    for (i = 0; i < 6; i++) {
      if (state[i] !== i) return { solved: false };
    }
    return { solved: true };
  }

  function createGame(cfg) {
    var seed = cfg && cfg.seed !== undefined ? Number(cfg.seed) : Date.now();
    var moves = scramble(seed);
    var st = SOLVED.slice();
    var i;
    for (i = 0; i < moves.length; i++) st = applyMove(st, moves[i]);
    return { state: st, scramble: moves };
  }

  function normalizeConfig(raw) {
    return { seed: raw && raw.seed !== undefined ? Number(raw.seed) : Date.now() };
  }

  function pyraminxStamp(cfg) {
    return "pyraminx:" + (cfg && cfg.seed !== undefined ? cfg.seed : "default");
  }

  globalThis.PyraminxCore = {
    createGame: createGame,
    applyMove: applyMove,
    verifySolved: verifySolved,
    normalizeConfig: normalizeConfig,
    pyraminxStamp: pyraminxStamp,
    DEFAULT_CONFIG: { seed: 1 },
  };
})();
