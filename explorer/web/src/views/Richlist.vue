<template>
  <div class="richlist-page">
    <h1>Rich List</h1>
    <p class="subtitle">Top addresses by ASRM balance</p>

    <div v-if="stats" class="supply-banner">
      <div class="supply-item">
        <span class="supply-label">Circulating Supply</span>
        <span class="supply-value">{{ formatAmount(stats.circulating_supply) }} ASRM</span>
      </div>
      <div class="supply-item">
        <span class="supply-label">Total Addresses</span>
        <span class="supply-value">{{ stats.total_addresses.toLocaleString() }}</span>
      </div>
    </div>

    <div v-if="loading" class="loading">Loading rich list...</div>

    <div v-else-if="error" class="error">{{ error }}</div>

    <div v-else>
      <!-- Distribution buckets -->
      <div class="distribution-section">
        <h2>Coin Distribution</h2>
        <div class="buckets-grid">
          <div v-for="bucket in distribution" :key="bucket.label" class="bucket-card">
            <div class="bucket-label">{{ bucket.label }}</div>
            <div class="bucket-count">{{ bucket.count }} addresses</div>
            <div class="bucket-bar-wrap">
              <div class="bucket-bar" :style="{ width: bucket.pct + '%' }"></div>
            </div>
            <div class="bucket-pct">{{ bucket.pct.toFixed(1) }}%</div>
          </div>
        </div>
      </div>

      <!-- Top addresses table -->
      <div class="table-section">
        <h2>Top {{ entries.length }} Addresses</h2>
        <div class="table-wrap">
          <table class="richlist-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Address</th>
                <th>Balance (ASRM)</th>
                <th>Share</th>
                <th>Share Bar</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="entry in entries"
                :key="entry.address"
                @click="goToAddress(entry.address)"
                class="row-link"
              >
                <td class="rank">{{ entry.rank }}</td>
                <td class="address">
                  <span class="addr-full">{{ entry.address }}</span>
                  <span class="addr-short">{{ truncateAddress(entry.address) }}</span>
                </td>
                <td class="balance">{{ formatAmount(entry.balance) }}</td>
                <td class="pct">{{ entry.percentage.toFixed(2) }}%</td>
                <td class="bar-cell">
                  <div class="pct-bar-wrap">
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
  data() {
    return {
      entries: [],
      stats: null,
      loading: true,
      error: null,
    };
  },
  computed: {
    distribution() {
      if (!this.entries.length) return [];
      const DIVISOR = BigInt("1000000000000000000");
      const buckets = [
        { label: "≥ 1,000,000 ASRM", min: 1_000_000, count: 0, pct: 0 },
        { label: "100,000 – 999,999 ASRM", min: 100_000, count: 0, pct: 0 },
        { label: "10,000 – 99,999 ASRM", min: 10_000, count: 0, pct: 0 },
        { label: "1,000 – 9,999 ASRM", min: 1_000, count: 0, pct: 0 },
        { label: "100 – 999 ASRM", min: 100, count: 0, pct: 0 },
        { label: "1 – 99 ASRM", min: 1, count: 0, pct: 0 },
        { label: "< 1 ASRM", min: 0, count: 0, pct: 0 },
      ];

      for (const entry of this.entries) {
        const asrm = Number(BigInt(entry.balance) / DIVISOR);
        for (let i = 0; i < buckets.length; i++) {
          if (asrm >= buckets[i].min) {
            buckets[i].count++;
            break;
          }
        }
      }

      const total = this.entries.length || 1;
      for (const b of buckets) {
        b.pct = (b.count / total) * 100;
      }

      return buckets.filter((b) => b.count > 0);
    },
  },
  mounted() {
    this.fetchData();
  },
  methods: {
    async fetchData() {
      try {
        this.loading = true;
        const [richlistRes, statsRes] = await Promise.all([
          explorerAPI.getRichlist(100),
          explorerAPI.getStats(),
        ]);
        this.entries = richlistRes.data.entries || [];
        this.stats = statsRes.data;
      } catch (err) {
        this.error = "Failed to load rich list data.";
        console.error(err);
      } finally {
        this.loading = false;
      }
    },
    formatAmount(value) {
      if (!value) return "0";
      let num;
      try {
        if (typeof value === "string" && value.startsWith("0x")) {
          num = BigInt(value);
        } else {
          num = BigInt(value);
        }
        const divisor = BigInt("1000000000000000000");
        const whole = num / divisor;
        const frac = num % divisor;
        const fracStr = frac.toString().padStart(18, "0").slice(0, 4).replace(/0+$/, "");
        const wholeStr = Number(whole).toLocaleString("en-US");
        return fracStr ? `${wholeStr}.${fracStr}` : wholeStr;
      } catch {
        return "0";
      }
    },
    truncateAddress(addr) {
      if (!addr || addr.length < 16) return addr;
      return addr.slice(0, 10) + "..." + addr.slice(-8);
    },
    goToAddress(address) {
      this.$router.push(`/address/${address}`);
    },
  },
};
</script>

<style scoped>
.richlist-page {
  width: 100%;
}

h1 {
  font-size: 2rem;
  color: #667eea;
  margin-bottom: 0.25rem;
}

.subtitle {
  color: #666;
  margin-bottom: 2rem;
}

.supply-banner {
  display: flex;
  gap: 2rem;
  flex-wrap: wrap;
  background: white;
  border-radius: 12px;
  padding: 1.25rem 2rem;
  box-shadow: 0 2px 8px rgba(0,0,0,0.08);
  margin-bottom: 2rem;
}

.supply-item {
  display: flex;
  flex-direction: column;
}

.supply-label {
  font-size: 0.8rem;
  color: #999;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.supply-value {
  font-size: 1.3rem;
  font-weight: bold;
  color: #333;
}

.loading, .error {
  text-align: center;
  padding: 3rem;
  color: #999;
}

.error {
  color: #e55;
}

.distribution-section {
  background: white;
  border-radius: 12px;
  padding: 1.5rem;
  box-shadow: 0 2px 8px rgba(0,0,0,0.08);
  margin-bottom: 2rem;
}

.distribution-section h2 {
  margin-bottom: 1.25rem;
  color: #333;
  border-bottom: 2px solid #667eea;
  padding-bottom: 0.5rem;
}

.buckets-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 1rem;
}

.bucket-card {
  background: #f8f9ff;
  border-radius: 8px;
  padding: 0.75rem 1rem;
}

.bucket-label {
  font-size: 0.8rem;
  color: #555;
  margin-bottom: 0.25rem;
  font-weight: 600;
}

.bucket-count {
  font-size: 1.1rem;
  font-weight: bold;
  color: #667eea;
  margin-bottom: 0.4rem;
}

.bucket-bar-wrap {
  background: #e8ebff;
  border-radius: 4px;
  height: 6px;
  margin-bottom: 0.25rem;
  overflow: hidden;
}

.bucket-bar {
  height: 100%;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  border-radius: 4px;
  transition: width 0.4s ease;
}

.bucket-pct {
  font-size: 0.75rem;
  color: #999;
}

.table-section {
  background: white;
  border-radius: 12px;
  padding: 1.5rem;
  box-shadow: 0 2px 8px rgba(0,0,0,0.08);
}

.table-section h2 {
  margin-bottom: 1.25rem;
  color: #333;
  border-bottom: 2px solid #667eea;
  padding-bottom: 0.5rem;
}

.table-wrap {
  overflow-x: auto;
}

.richlist-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.9rem;
}

.richlist-table th {
  text-align: left;
  padding: 0.6rem 1rem;
  background: #f8f9ff;
  color: #555;
  font-weight: 600;
  border-bottom: 2px solid #eef;
}

.richlist-table td {
  padding: 0.6rem 1rem;
  border-bottom: 1px solid #f0f0f0;
  vertical-align: middle;
}

.row-link {
  cursor: pointer;
  transition: background 0.15s;
}

.row-link:hover {
  background: #f8f9ff;
}

.rank {
  color: #999;
  width: 3rem;
  text-align: center;
}

.address {
  font-family: monospace;
  color: #667eea;
}

.addr-short {
  display: none;
}

.balance {
  text-align: right;
  font-weight: 600;
  color: #333;
}

.pct {
  text-align: right;
  color: #764ba2;
  font-weight: 600;
  width: 6rem;
}

.bar-cell {
  width: 10rem;
  padding-right: 0.5rem;
}

.pct-bar-wrap {
  background: #e8ebff;
  border-radius: 4px;
  height: 8px;
  overflow: hidden;
}

.pct-bar {
  height: 100%;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  border-radius: 4px;
  transition: width 0.4s ease;
  min-width: 2px;
}

@media (max-width: 768px) {
  .addr-full {
    display: none;
  }
  .addr-short {
    display: inline;
  }
  .bar-cell {
    display: none;
  }
  .buckets-grid {
    grid-template-columns: 1fr 1fr;
  }
}
</style>
