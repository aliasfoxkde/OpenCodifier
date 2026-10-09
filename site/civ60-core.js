/* OpenCodifier — AI Civilization in 60 Seconds, core (DOM-free).
   §5.13 of the Decisions-SDK plan: decision propagation. Citizens decide
   from ten behaviors — work / farm / build / trade / explore / fight /
   eat / sleep / have-child / flee — driven by needs and events, and the
   player injects events (food shortage, raid, harvest, festival,
   migrants) to watch the cascade: shortage → hunger ↑ → farming ↑ →
   wood production ↓ → construction slows.

   Rule prefilters, exactly as in the ant colony:
     - sleep: energy ≤ 15 is ALWAYS a rule — never a candidate
     - eat:   hunger ≥ 80 with food on hand is ALWAYS a rule — the
              contested "eat" candidate only exists as rationing
              (hunger 40–80, or no food left)
     - have-child: surplus food + a healthy citizen is a rule
   Everything else is scored on the snapshot (food deficit, wood need,
   homeless share, raid pressure), and genuine near-ties within TIE_EPS
   go to the engine — decisions batched like MicroWorld: the city holds
   ask-tokens refilled at cfg.askBudget per sim-second, and a citizen
   with no token takes the argmax rule with the rung saying so.

   Population is coherent by construction:
     pop = start + births + migrants − deaths − fled,
   and runSummary carries every term plus the boolean.

   The shell (civ60.js) renders; this file never touches the DOM. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const TICKS_PER_S = 60;
  const DECIDE_EVERY = 45;            /* ticks — each citizen decides ~1.33 Hz */
  const TIE_EPS = 120;
  const POP_CAP = 200;
  const START = { citizens: 12, food: 40, wood: 10, homes: 2 };
  const HOME_COST = 12;               /* wood per home — a project, not a snack */
  const HUNGER_RATE = 1.1;            /* per sim-second */
  const HUNGER_SLEEP_RATE = 0.5;
  const ENERGY_RATE = 0.9;            /* drain while working/farming/building */
  const ENERGY_SLEEP = 5;             /* recover while sleeping */
  const STARVE_TICKS = 30 * TICKS_PER_S;   /* at hunger 100 → death */
  const FARM_RATE = 0.5;              /* food/s while farming */
  const WORK_RATE = 0.4;              /* wood/s while working */
  const EXPLORE_TIME = 12 * TICKS_PER_S;   /* explore yield after this long */
  const RAID_LENGTH = 20 * TICKS_PER_S;
  const RAID_TICKS = 2 * TICKS_PER_S;      /* a loss every this many ticks */
  const RAIDERS = 3;
  const MIGRANTS = 3;
  const FLEE_FEAR = 85;
  const SLEEP_ENERGY = 15;            /* energy ≤ this → sleep is a rule */
  const EAT_HUNGER = 80;              /* hunger ≥ this with food → eat is a rule */

  const BEHAVIORS = ["work", "farm", "build", "trade", "explore",
    "fight", "eat", "flee"];
  const RULE_ONLY = ["sleep", "have-child"];     /* never candidates */
  const BEHAVIOR_DESC = {
    work: "fell timber — the wood stock is the city's construction budget",
    farm: "work the fields — food now beats everything while hunger bites",
    build: "raise a home from the wood stock for a homeless citizen",
    trade: "swap surplus food for wood at the market stall",
    explore: "send a scout past the fields for a discovery yield",
    fight: "take up the guard line against the raiders",
    eat: "take a ration — hunger is climbing and the granary is thin",
    flee: "run for the treeline while the raid is on",
  };
  const EVENTS = ["shortage", "harvest", "raid", "festival", "migrants"];
  const DEFAULT_CONFIG = {
    seed: "oc-civ", citizens: START.citizens, askBudget: 2, events: true,
  };

  /* ---------- config + stamp ---------- */

  function normalizeConfig(raw) {
    const c = raw || {};
    return {
      seed: c.seed === undefined || c.seed === null ? DEFAULT_CONFIG.seed : String(c.seed),
      citizens: DK.clamp(Math.round(Number(c.citizens) || DEFAULT_CONFIG.citizens), 4, 60),
      /* 0 is a meaningful budget (rules-only city): explicit check, no `||` */
      askBudget: (c.askBudget === undefined || c.askBudget === null
        || Number.isNaN(Number(c.askBudget)))
        ? DEFAULT_CONFIG.askBudget
        : DK.clampf(Number(c.askBudget), 0, 10, DEFAULT_CONFIG.askBudget),
      events: c.events === undefined ? true : !!c.events,
    };
  }

  const civStamp = (cfg) =>
    "civ|" + cfg.seed + "|" + cfg.citizens + "|" + cfg.askBudget + "|" + cfg.events;

  /* ---------- world ---------- */

  function makeCitizen(i, rng) {
    return {
      id: "c" + i,
      hunger: 15 + rng() * 20,
      energy: 60 + rng() * 40,
      fear: 0,
      behavior: "rest",
      starve: 0,                       /* consecutive ticks at hunger 100 */
      decideIn: Math.floor(rng() * DECIDE_EVERY),
      exploreT: 0,
    };
  }

  function createGame(raw) {
    const cfg = normalizeConfig(raw);
    const rng = DK.makeRng("civ:" + cfg.seed);
    const citizens = [];
    for (let i = 0; i < cfg.citizens; i++) citizens.push(makeCitizen(i, rng));
    return {
      cfg, stamp: civStamp(cfg), rng,
      citizens, nextId: cfg.citizens,
      food: START.food, wood: START.wood, homes: START.homes,
      ticks: 0,
      decisions: 0, rungs: { rule: 0, engine: 0 },
      engineCalls: 0, tokens: cfg.askBudget,
      tally: {},                        /* every behavior, rule and engine alike */
      births: 0, deaths: 0, fled: 0, migrants: 0, repelled: 0,
      raid: null,                       /* {left, nextLoss, raiders} while a raid runs */
      festival: 0,                      /* ticks of morale left */
      log: [],                          /* {tick, kind, note} */
      history: [],                      /* {t, food, wood, homes, pop} every 5 s */
      discovered: 0,
      over: false, lastDecision: null,
    };
  }

  const pop = (g) => g.citizens.length;
  /* the food pull is the WORSE of granary and bellies: a full granary with
     hungry citizens still means farm (the cascade needs hunger to bite) */
  function foodDeficit(g) {
    const granary = 1 - g.food / Math.max(4, pop(g) * 2);
    let hungerSum = 0;
    for (const c of g.citizens) hungerSum += c.hunger;
    const bellies = hungerSum / Math.max(1, pop(g)) / 100;
    return DK.clampf(Math.max(granary, bellies), 0, 1, 0);
  }
  const homeless = (g) => Math.max(0, pop(g) - g.homes);
  const woodNeed = (g) => homeless(g) > 0 ? 1 : DK.clampf(1 - g.wood / 40, 0, 1, 0);
  const defenders = (g) => g.citizens.filter((c) => c.behavior === "fight").length;

  /* ---------- events ---------- */

  function injectEvent(g, kind) {
    if (!EVENTS.includes(kind) || g.over) return null;
    const t = g.ticks;
    if (kind === "shortage") {
      /* a granary loss sized to hurt: ~4 days of food per citizen, so the
         bellies term takes over and the farm cascade is visible */
      const lost = Math.min(Math.round(g.food), pop(g) * 4);
      g.food -= lost;
      g.log.push({ tick: t, kind, note: "granary spoiled: -" + lost + " food" });
    } else if (kind === "harvest") {
      const gain = 8 * pop(g);
      g.food += gain;
      g.log.push({ tick: t, kind, note: "bountiful harvest: +" + gain + " food" });
    } else if (kind === "raid") {
      if (g.raid) return null;
      g.raid = { left: RAID_LENGTH, nextLoss: RAID_TICKS, raiders: RAIDERS };
      for (const c of g.citizens) c.fear = Math.min(100, c.fear + 40);
      g.log.push({ tick: t, kind, note: RAIDERS + " raiders at the palisade" });
    } else if (kind === "festival") {
      g.festival = 30 * TICKS_PER_S;
      for (const c of g.citizens) {
        c.fear = 0;
        c.energy = Math.min(100, c.energy + 20);
        c.hunger = Math.max(0, c.hunger - 10);
      }
      g.log.push({ tick: t, kind, note: "festival: fear cleared, spirits up" });
    } else if (kind === "migrants") {
      for (let i = 0; i < MIGRANTS && pop(g) < POP_CAP; i++) {
        g.citizens.push(makeCitizen(g.nextId++, g.rng));
        g.migrants += 1;
      }
      g.log.push({ tick: t, kind, "note": MIGRANTS + " migrants joined the city" });
    }
    return kind;
  }

  /* ---------- one-shot behavior effects (called from settle) ---------- */

  function doBuild(g) {
    if (g.wood >= HOME_COST && homeless(g) > 0) {
      g.wood -= HOME_COST;
      g.homes += 1;
    }
  }

  function doTrade(g) {
    if (g.food >= 3) {
      g.food -= 3;
      g.wood += 3;
    }
  }

  function doEat(g, c) {
    if (g.food >= 1) {
      g.food -= 1;
      c.hunger = Math.max(0, c.hunger - 45);
    }
  }

  function doChild(g) {
    if (g.food >= 2 && pop(g) < POP_CAP) {
      g.food -= 2;
      g.citizens.push(makeCitizen(g.nextId++, g.rng));
      g.births += 1;
      g.log.push({ tick: g.ticks, kind: "birth", note: "a child was born" });
    }
  }

  function doFlee(g, c) {
    const i = g.citizens.indexOf(c);
    if (i !== -1) {
      g.citizens.splice(i, 1);
      g.fled += 1;
      g.log.push({ tick: g.ticks, kind: "flee", note: c.id + " fled the raid" });
    }
  }

  /* ---------- the decision ladder ---------- */

  function settle(g, c, behavior, rung, evGap) {
    if (behavior === "build") doBuild(g);
    else if (behavior === "trade") doTrade(g);
    else if (behavior === "eat") doEat(g, c);
    else if (behavior === "have-child") doChild(g);
    else if (behavior === "flee") doFlee(g, c);
    c.behavior = behavior;
    g.tally[behavior] = (g.tally[behavior] || 0) + 1;
    g.decisions += 1;
    g.rungs[rung] += 1;
    g.lastDecision = {
      id: c.id, choice: behavior, rung,
      evGap: rung === "rule" && evGap === undefined ? null : evGap,
      at: { hunger: Math.round(c.hunger), energy: Math.round(c.energy),
        fear: Math.round(c.fear) },
    };
    return { behavior, rung, evGap };
  }

  function decide(g, c) {
    c.decideIn = DECIDE_EVERY;
    if (g.over) return null;

    /* RULE PREFILTERS — the never-asked paths */
    if (c.energy <= SLEEP_ENERGY) return settle(g, c, "sleep", "rule");
    if (c.hunger >= EAT_HUNGER && g.food >= 1) return settle(g, c, "eat", "rule");
    /* children need roofs: births gate on spare housing, which puts BUILD
       upstream of growth. HOME_COST 12 makes wood the rate-limiter, the
       slack-4 gate spreads births out — a 2-minute run stays a town. */
    if (g.food > pop(g) * 3 && c.hunger < 40 && pop(g) < POP_CAP
      && pop(g) < g.homes + 4 && g.rng() < 0.15) {
      return settle(g, c, "have-child", "rule");
    }

    /* scored candidates on the snapshot */
    const cands = [];
    const fd = foodDeficit(g);
    cands.push({ id: "farm", ev: Math.round(300 + 500 * fd) });
    cands.push({ id: "work", ev: Math.round(250 + 300 * woodNeed(g)) });
    if (g.wood >= HOME_COST && homeless(g) > 0) {
      /* base 500 keeps BUILD above WORK (550 cap) while homelessness is
         material — otherwise the city works forever and never houses the
         birth gate. A 1/12 share still loses to work: the last home waits
         for the next birth, which is the self-balancing loop. */
      cands.push({ id: "build", ev: Math.round(500 + 350 * (homeless(g) / pop(g))) });
    }
    if (g.food > pop(g) * 2 && g.wood < 30) {
      cands.push({ id: "trade", ev: 300 });
    }
    cands.push({ id: "explore", ev: Math.round(250 + 150 * (1 - fd)) });
    if (g.raid) {
      cands.push({ id: "fight",
        ev: Math.round(500 + 200 * Math.max(0, g.raid.raiders - defenders(g))) });
    }
    if (c.hunger >= 40) {
      /* rationing: urgent eat is a rule above; this is the thin-granary case */
      cands.push({ id: "eat",
        ev: Math.round(350 + 300 * (c.hunger - 40) / 40 - (g.food >= 1 ? 0 : 150)) });
    }
    if (g.raid && c.fear >= FLEE_FEAR) {
      cands.push({ id: "flee", ev: Math.round(400 + 300 * c.fear / 100) });
    }
    cands.sort((a, b) => b.ev - a.ev || BEHAVIORS.indexOf(a.id) - BEHAVIORS.indexOf(b.id));

    const evGap = cands.length > 1 ? cands[0].ev - cands[1].ev : Infinity;
    let pick = cands[0].id;
    let rung = "rule";

    /* genuine contest + a token in the batch budget + a live bridge */
    if (cands.length > 1 && evGap <= TIE_EPS && g.tokens >= 1 && g.engineChoose && !g.over) {
      g.tokens -= 1;
      g.engineCalls += 1;
      const req = {
        state: {
          text: "Citizen " + c.id + " of a city of " + pop(g) +
            " at tick " + g.ticks + ". Hunger " + Math.round(c.hunger) +
            "/100, energy " + Math.round(c.energy) + "/100, fear " +
            Math.round(c.fear) + "/100. City stocks: food " + Math.round(g.food) +
            ", wood " + Math.round(g.wood) + ", homes " + g.homes +
            " for " + pop(g) + " citizens" +
            (g.raid ? ". A RAID is running: " + defenders(g) + " defenders against " +
              g.raid.raiders + " raiders" : ". No raid") +
            ". Pick this citizen's next behavior.",
          facts: {},
        },
        questions: [{
          type: "choice", id: "citizen-behavior",
          text: "What should citizen " + c.id + " do next?",
          candidates: cands.map((cd) => ({ id: cd.id, description: BEHAVIOR_DESC[cd.id] })),
        }],
        policy: { min_confidence: 0.55, verify_below: 0.4, abstain_below: 0.3, risk: "low" },
      };
      const answer = g.engineChoose ? g.engineChoose(req) : null;
      const ans = answer && answer.outcome === "accept" && Array.isArray(answer.answers)
        ? answer.answers.find((a) => a && typeof a.choice === "string") : null;
      if (ans && cands.some((cd) => cd.id === ans.choice)) {
        pick = ans.choice;
        rung = "engine";
      }
    }
    return settle(g, c, pick, rung, evGap);
  }

  /* ---------- per-tick behavior effects ---------- */

  function applyBehavior(g, c, dt) {
    const b = c.behavior;
    if (b === "sleep") {
      c.energy = Math.min(100, c.energy + ENERGY_SLEEP * dt);
    } else if (b === "farm") {
      g.food += FARM_RATE * dt * (g.festival > 0 ? 1.25 : 1);
      c.energy = Math.max(0, c.energy - ENERGY_RATE * dt);
    } else if (b === "work") {
      g.wood += WORK_RATE * dt * (g.festival > 0 ? 1.25 : 1);
      c.energy = Math.max(0, c.energy - ENERGY_RATE * dt);
    } else if (b === "explore") {
      c.exploreT += dt;
      if (c.exploreT * TICKS_PER_S >= EXPLORE_TIME) {
        const found = Math.round(6 + g.rng() * 8);
        g.food += found;
        g.discovered += found;
        c.exploreT = 0;
        g.log.push({ tick: g.ticks, kind: "explore",
          note: c.id + " scouted a berry stand: +" + found + " food" });
      }
      c.energy = Math.max(0, c.energy - ENERGY_RATE * 0.5 * dt);
    }
    /* eat / have-child / build / trade / fight: one-shot effects applied
       at settle time; fight holds the guard line (defenders()) */
  }

  function raidTick(g, dt) {
    if (!g.raid) return;
    const r = g.raid;
    r.left -= dt * TICKS_PER_S;
    const df = defenders(g);
    if (df >= r.raiders) {
      g.repelled += 1;
      g.raid = null;
      for (const c of g.citizens) c.fear = Math.max(0, c.fear - 30);
      g.log.push({ tick: g.ticks, kind: "raid",
        note: "raid repelled by the guard line (" + df + " defenders)" });
      return;
    }
    r.nextLoss -= dt * TICKS_PER_S;
    if (r.nextLoss <= 0) {
      r.nextLoss = RAID_TICKS;
      const victim = g.citizens[Math.floor(g.rng() * g.citizens.length)];
      if (victim) {
        g.citizens.splice(g.citizens.indexOf(victim), 1);
        g.deaths += 1;
        g.log.push({ tick: g.ticks, kind: "raid",
          note: victim.id + " fell to the raiders" });
      }
      g.food = Math.max(0, g.food - 5);
    }
    if (r.left <= 0) {
      g.raid = null;
      for (const c of g.citizens) c.fear = Math.max(0, c.fear - 30);
      g.log.push({ tick: g.ticks, kind: "raid", note: "the raiders gave up and left" });
    }
  }

  function tickGame(g, engineChoose) {
    if (g.over) return;
    const dt = 1 / TICKS_PER_S;
    g.ticks += 1;
    g.engineChoose = engineChoose || null;
    g.tokens = Math.min(g.cfg.askBudget, g.tokens + g.cfg.askBudget * dt);
    if (g.festival > 0) g.festival -= 1;

    raidTick(g, dt);

    /* iterate a copy: flee and raid deaths remove citizens mid-tick */
    for (const c of g.citizens.slice()) {
      if (g.citizens.indexOf(c) === -1) continue;      /* fled or died above */
      c.decideIn -= 1;
      if (c.decideIn <= 0) decide(g, c);
      if (g.citizens.indexOf(c) === -1) continue;      /* fled in settle */
      applyBehavior(g, c, dt);
      c.hunger = Math.min(100, c.hunger + HUNGER_RATE * dt);
      if (c.hunger >= 100) {
        c.starve += 1;
        if (c.starve >= STARVE_TICKS) {
          g.citizens.splice(g.citizens.indexOf(c), 1);
          g.deaths += 1;
          g.log.push({ tick: g.ticks, kind: "starve", note: c.id + " starved" });
        }
      } else {
        c.starve = 0;
      }
    }

    if (g.ticks % (5 * TICKS_PER_S) === 0) {
      g.history.push({ t: Math.round(g.ticks / TICKS_PER_S),
        food: Math.round(g.food), wood: Math.round(g.wood),
        homes: g.homes, pop: pop(g) });
      if (g.history.length > 400) g.history.shift();
    }

    if (pop(g) === 0) {
      g.over = true;
      g.log.push({ tick: g.ticks, kind: "collapse", note: "the city is empty" });
    }
  }

  /* ---------- summary ---------- */

  /* coherence: pop = start + births + migrants − deaths − fled */
  function coherentPop(g) {
    return g.cfg.citizens + g.births + g.migrants - g.deaths - g.fled;
  }

  function runSummary(g) {
    const simS = g.ticks / TICKS_PER_S;
    return {
      ticks: g.ticks, simSeconds: +simS.toFixed(1),
      decisions: g.decisions,
      perSecond: simS ? +(g.decisions / simS).toFixed(2) : 0,
      rungs: Object.assign({}, g.rungs),
      engineCalls: g.engineCalls,
      tally: Object.assign({}, g.tally),
      pop: pop(g), homes: g.homes,
      food: Math.round(g.food), wood: Math.round(g.wood),
      births: g.births, deaths: g.deaths, fled: g.fled,
      migrants: g.migrants, repelled: g.repelled, discovered: g.discovered,
      events: g.log.length, over: g.over,
      coherent: pop(g) === coherentPop(g),
      history: g.history.length,
    };
  }

  function simulateTicks(g, ticks, engineChoose) {
    for (let i = 0; i < ticks; i++) {
      if (g.over) break;
      tickGame(g, engineChoose);
    }
    return g;
  }

  globalThis.CivCore = {
    TICKS_PER_S, DECIDE_EVERY, TIE_EPS, POP_CAP, START, HOME_COST, STARVE_TICKS,
    HUNGER_RATE, SLEEP_ENERGY, EAT_HUNGER, FLEE_FEAR,
    RAID_LENGTH, RAIDERS, MIGRANTS,
    BEHAVIORS, RULE_ONLY, BEHAVIOR_DESC, EVENTS, DEFAULT_CONFIG,
    normalizeConfig, civStamp, createGame, decide, tickGame, simulateTicks,
    injectEvent, runSummary, defenders, foodDeficit, homeless, pop,
    coherentPop,
  };
})();
