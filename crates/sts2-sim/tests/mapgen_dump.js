'use strict';
// Emit reference map outputs (point-type-per-coord + RNG counter) for
// several act/seed/ascension combinations from the dashboard's JS mapgen
// port. The Rust StandardActMap port must reproduce these byte-for-byte.
//
// Run via:  node mapgen_dump.js > mapgen_reference.json

const path = require('path');
const MAPGEN_DIR = 'C:\\Users\\zhuyl\\OneDrive\\Desktop\\sts2_stats\\Release Version\\scripts\\mapgen';
const { Rng }                  = require(path.join(MAPGEN_DIR, 'rng.js'));
const { buildStandardActMap }  = require(path.join(MAPGEN_DIR, 'generator.js'));
const { pruneAndRepair }       = require(path.join(MAPGEN_DIR, 'pruning.js'));
const { getActConfig, ascensionFlags, defaultNumOfElites } =
    require(path.join(MAPGEN_DIR, 'act_config.js'));
const { getDeterministicHashCode } = require(path.join(MAPGEN_DIR, 'string_hash.js'));

function dumpOne({ actId, actIndex, seedString, ascension, isMultiplayer, prune }) {
  const cfg = getActConfig(actId);
  const seedU32 = getDeterministicHashCode(seedString) >>> 0;
  const flags = ascensionFlags(ascension || 0);
  const numOfElites = defaultNumOfElites(flags.swarmingElites);
  const hasSecondBoss = (actId === 'ACT.GLORY') && flags.doubleBoss;
  const rng = new Rng(seedU32, `act_${actIndex + 1}_map`);

  const graph = buildStandardActMap({
    cfg, rng,
    isMultiplayer: !!isMultiplayer,
    hasSecondBoss,
    replaceTreasureWithElites: false,
    numOfElites,
  });
  if (prune) pruneAndRepair(graph);

  // Pull out (col, row, point_type, parents, children) per node.
  const nodes = [];
  for (let c = 0; c < graph.grid.length; c++) {
    for (let r = 0; r < graph.grid[c].length; r++) {
      const p = graph.grid[c][r];
      if (!p) continue;
      nodes.push({
        col: p.coord.col, row: p.coord.row,
        point_type: p.PointType,
        parents: [...p.parents].map(x => [x.coord.col, x.coord.row]),
        children: [...p.Children].map(x => [x.coord.col, x.coord.row]),
      });
    }
  }
  // Sort for deterministic output.
  nodes.sort((a, b) => (a.col - b.col) || (a.row - b.row));
  for (const n of nodes) {
    n.parents.sort((a, b) => (a[0] - b[0]) || (a[1] - b[1]));
    n.children.sort((a, b) => (a[0] - b[0]) || (a[1] - b[1]));
  }
  return {
    actId, actIndex, seedString, ascension, isMultiplayer, prune,
    seedU32, mapLength: graph.mapLength,
    pointTypeCounts: graph.pointTypeCounts,
    rngCounterAfter: rng.Counter,
    nodes,
  };
}

const SCENARIOS = [
  // (actId, actIndex, seedString, ascension, isMultiplayer, prune)
  { actId: 'ACT.OVERGROWTH', actIndex: 0, seedString: 'TEST',     ascension: 0, isMultiplayer: false, prune: false },
  { actId: 'ACT.OVERGROWTH', actIndex: 0, seedString: 'TEST',     ascension: 0, isMultiplayer: false, prune: true  },
  { actId: 'ACT.OVERGROWTH', actIndex: 0, seedString: 'BEEFCAFE', ascension: 0, isMultiplayer: false, prune: true  },
  { actId: 'ACT.HIVE',       actIndex: 1, seedString: 'TEST',     ascension: 0, isMultiplayer: false, prune: true  },
  { actId: 'ACT.GLORY',      actIndex: 2, seedString: 'TEST',     ascension: 0, isMultiplayer: false, prune: true  },
  { actId: 'ACT.UNDERDOCKS', actIndex: 3, seedString: 'TEST',     ascension: 0, isMultiplayer: false, prune: true  },
  { actId: 'ACT.OVERGROWTH', actIndex: 0, seedString: 'TEST',     ascension: 1, isMultiplayer: false, prune: true  },
];

console.log(JSON.stringify(SCENARIOS.map(dumpOne), null, 2));
