<template>
  <div class="page">
    <div class="page-header">
      <h1>Rich List</h1>
      <span class="page-sub">Top addresses by ASRM balance</span>
    </div>

    <!-- Supply banner -->
    <div v-if="stats" class="supply-row">
      <div class="supply-item">
        <span class="supply-label">Circulating Supply</span>
        <span class="supply-val green-text">{{ formatAmount(stats.circulating_supply) }} <span class="unit">ASRM</span></span>
      </div>
      <div class="supply-item">
        <span class="supply-label">Total Addresses</span>
        <span class="supply-val accent-text">{{ stats.total_addresses.toLocaleString() }}</span>
      </div>
      <div class="supply-item">
        <span class="supply-label">Total Blocks</span>
        <span class="supply-val">{{ stats.total_blocks.toLocaleString() }}</span>
      </div>
    </div>

    <div v-if="loading" class="empty"><span class="spin">⟳</span> Loading rich list...</div>
    <div v-else-if="error" class="empty red-text">{{ error }}</div>

    <div v-else>
      <!-- Distribution -->
      <div class="card dist-card">
        <div class="card-header">
          <span class="card-title">Coin Distribution</span>
        </div>
        <div class="buckets">
          <div v-for="b in distribution" :key="b.label" class="bucket">
            <div class="bucket-top">
              <span class="bucket-label">{{ b.label }}</span>
              <span class="bucket-count accent-text">{{ b.count }}</span>
            </div>
            <div class="bucket-bar-bg">
              <div class="bucket-bar" :style="{ width: b.pct + '%' }"></div>
            </div>
            <div class="bucket-pct">{{ b.pct.toFixed(1) }}%</div>
          </div>
        </div>
      </div>

      <!-- Table -->
      <div class="card table-card">
        <div class="card-header">
          <span class="card-title">Top {{ entries.length }} Addresses</span>
        </div>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th class="center">#</th>
                <th>Address</th>
                <th class="right">Balance (ASRM)</th>
                <th class="right">Share</th>
                <th>Distribution</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="entry in entries"
                :key="entry.address"
                @click="$router.push(`/address/${entry.address}`)"
                class="row-link"
              >
                <td class="center rank">{{ entry.rank }}</td>
                <td>
                  <span class="mono accent-text">{{ entry.address }}</span>
                </td>
                <td class="right green-text bold">{{ formatAmount(entry.balance) }}</td>
                <td class="right purple-text bold">{{ entry.percentage.toFixed(2) }}%</td>
                <td class="bar-cell">
                  <div class="pct-bar-bg">
                    <div class="pct-bar" :style="{ width: Math.min(entry.percentage, 100) + '%' }"></div>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";
export default {
  name: "Richlist",
  data() { return { entries: [], stats: null, loading: true, error: null }; },
  computed: {
    distribution() {
      if (!this.entries.length) return [];
      const D = BigInt("1000000000000000000");
      const buckets = [
        { label: "≥ 1M ASRM",      min: 1_000_000, count: 0, pct: 0 },
        { label: "100K–999K ASRM", min: 100_000,   count: 0, pct: 0 },
        { label: "10K–99K ASRM",   min: 10_000,    count: 0, pct: 0 },
        { label: "1K–9K ASRM",     min: 1_000,     count: 0, pct: 0 },
        { label: "100–999 ASRM",   min: 100,        count: 0, pct: 0 },
        { label: "1–99 ASRM",      min: 1,          count: 0, pct: 0 },
        { label: "< 1 ASRM",       min: 0,          count: 0, pct: 0 },
      ];
      for (const e of this.entries) {
        try {
          const asrm = Number(BigInt(e.balance) / D);
          for (const b of buckets) { if (asrm >= b.min) { b.count++; break; } }
        } catch {}
      }
      const total = this.entries.length || 1;
      for (const b of buckets) b.pct = (b.count / total) * 100;
      return buckets.filter(b => b.count > 0);
    },
  },
  mounted() { this.fetch(); },
  methods: {
    async fetch() {
      try {
        this.loading = true;
        const [rl, st] = await Promise.all([explorerAPI.getRichlist(100), explorerAPI.getStats()]);
        this.entries = rl.data.entries || [];
        this.stats = st.data;
      } catch (e) {
        this.error = "Failed to load rich list.";
      } finally { this.loading = false; }
    },
    formatAmount(value) {
      if (!value) return "0";
      try {
        const n = BigInt(value);
        const D = BigInt("1000000000000000000");
        const whole = n / D;
        const frac = n % D;
        const fracStr = frac.toString().padStart(18, "0").slice(0, 4).replace(/0+$/, "");
        const wholeStr = Number(whole).toLocaleString("en-US");
        return fracStr ? `${wholeStr}.${fracStr}` : wholeStr;
      } catch { return "0"; }
    },
  },
};
</script>

<style scoped>
.page { width: 100%; }

.page-header {
  display: flex;
  align-items: baseline;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

h1 { font-size: 1.5rem; font-weight: 700; color: var(--text); }

.page-sub { font-size: 13px; color: var(--text2); }

/* Supply Row */
.supply-row {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 1px;
  background: var(--border);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
  margin-bottom: 1.5rem;
}

.supply-item {
  background: var(--surface);
  padding: 1.25rem 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.supply-label {
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--text2);
}

.supply-val {
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--text);
}

.unit { font-size: 0.7rem; font-weight: 400; color: var(--muted); }
.accent-text { color: var(--accent); }
.green-text { color: var(--green); }
.purple-text { color: #a78bfa; }
.red-text { color: var(--red); }
.bold { font-weight: 600; }

/* Cards */
.card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
  margin-bottom: 1.5rem;
}

.card-header {
  padding: 0.875rem 1.5rem;
  border-bottom: 1px solid var(--border);
}

.card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

/* Distribution */
.buckets {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 0;
}

.bucket {
  padding: 1.25rem 1.5rem;
  border-right: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
}

.bucket:last-child { border-right: none; }

.bucket-top {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 0.6rem;
}

.bucket-label { font-size: 11px; color: var(--text2); font-weight: 500; }
.bucket-count { font-size: 1.1rem; font-weight: 700; }

.bucket-bar-bg {
  background: var(--border2);
  border-radius: 4px;
  height: 5px;
  overflow: hidden;
  margin-bottom: 0.3rem;
}

.bucket-bar {
  height: 100%;
  background: linear-gradient(90deg, var(--accent), var(--accent2));
  border-radius: 4px;
  transition: width 0.4s ease;
}

.bucket-pct { font-size: 11px; color: var(--muted); }

/* Table */
.table-wrap { overflow-x: auto; }

table { width: 100%; border-collapse: collapse; }
thead tr { border-bottom: 1px solid var(--border); }

th {
  padding: 0.75rem 1.25rem;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--text2);
  text-align: left;
  white-space: nowrap;
}

td {
  padding: 0.75rem 1.25rem;
  font-size: 13px;
  border-bottom: 1px solid var(--border);
  color: var(--text);
}

tbody tr:last-child td { border-bottom: none; }
.row-link { cursor: pointer; transition: background 0.15s; }
.row-link:hover td { background: var(--surface2); }

.center { text-align: center; }
.right { text-align: right; }
.mono { font-family: var(--mono); font-size: 12px; }

.rank { color: var(--muted); font-size: 12px; width: 3rem; }

.bar-cell { width: 120px; }

.pct-bar-bg {
  background: var(--border2);
  border-radius: 4px;
  height: 6px;
  overflow: hidden;
}

.pct-bar {
  height: 100%;
  background: linear-gradient(90deg, var(--accent), var(--accent2));
  border-radius: 4px;
  min-width: 2px;
  transition: width 0.4s ease;
}

.empty {
  padding: 4rem;
  text-align: center;
  color: var(--muted);
  font-size: 13px;
}

.spin { display: inline-block; animation: spin 1.5s linear infinite; margin-right: 6px; }
@keyframes spin { to { transform: rotate(360deg); } }

@media (max-width: 768px) {
  .bar-cell { display: none; }
  .buckets { grid-template-columns: 1fr 1fr; }
}
</style>
