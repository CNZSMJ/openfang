// OpenFang Sessions Page — Session listing + Memory tab
'use strict';

function sessionsPage() {
  return {
    tab: 'sessions',
    // -- Sessions state --
    sessions: [],
    searchFilter: '',
    loading: true,
    loadError: '',

    // -- Memory state --
    memAgentId: '',
    kvPairs: [],
    memSearch: '',
    memNamespace: '',
    memLifecycle: '',
    memTags: '',
    memIncludeInternal: false,
    memCleanupLimit: 50,
    memCleanupLoading: false,
    memCleanupError: '',
    memCleanupResult: null,
    memCleanupRanAt: '',
    showAdd: false,
    newKey: '',
    newValue: '""',
    newKind: '',
    newTags: '',
    newFreshness: '',
    editingKey: null,
    editingValue: '',
    editingKind: '',
    editingTags: '',
    editingFreshness: '',
    memLoading: false,
    memLoadError: '',

    // -- Sessions methods --
    async loadSessions() {
      this.loading = true;
      this.loadError = '';
      try {
        var data = await OpenFangAPI.get('/api/sessions');
        var sessions = data.sessions || [];
        var agents = Alpine.store('app').agents;
        var agentMap = {};
        agents.forEach(function(a) { agentMap[a.id] = a.name; });
        sessions.forEach(function(s) {
          s.agent_name = agentMap[s.agent_id] || '';
        });
        this.sessions = sessions;
      } catch(e) {
        this.sessions = [];
        this.loadError = e.message || 'Could not load sessions.';
      }
      this.loading = false;
    },

    async loadData() { return this.loadSessions(); },

    get filteredSessions() {
      var f = this.searchFilter.toLowerCase();
      if (!f) return this.sessions;
      return this.sessions.filter(function(s) {
        return (s.agent_name || '').toLowerCase().indexOf(f) !== -1 ||
               (s.agent_id || '').toLowerCase().indexOf(f) !== -1;
      });
    },

    openInChat(session) {
      var agents = Alpine.store('app').agents;
      var agent = agents.find(function(a) { return a.id === session.agent_id; });
      if (agent) {
        Alpine.store('app').pendingAgent = agent;
      }
      location.hash = 'agents';
    },

    deleteSession(sessionId) {
      var self = this;
      OpenFangToast.confirm('Delete Session', 'This will permanently remove the session and its messages.', async function() {
        try {
          await OpenFangAPI.del('/api/sessions/' + sessionId);
          self.sessions = self.sessions.filter(function(s) { return s.session_id !== sessionId; });
          OpenFangToast.success('Session deleted');
        } catch(e) {
          OpenFangToast.error('Failed to delete session: ' + e.message);
        }
      });
    },

    // -- Memory methods --
    parseMemoryTags(raw) {
      return String(raw || '')
        .split(',')
        .map(function(tag) { return tag.trim(); })
        .filter(Boolean);
    },

    buildMemoryPayload(value, kind, tags, freshness) {
      var payload = { value: value };
      if (kind) payload.kind = kind;
      var parsedTags = this.parseMemoryTags(tags);
      if (parsedTags.length) payload.tags = parsedTags;
      if (freshness) payload.freshness = freshness;
      return payload;
    },

    resetAddForm() {
      this.newKey = '';
      this.newValue = '""';
      this.newKind = '';
      this.newTags = '';
      this.newFreshness = '';
    },

    buildMemoryQuery() {
      var params = new URLSearchParams();
      if (this.memNamespace) params.set('namespace', this.memNamespace);
      if (this.memLifecycle) params.set('lifecycle', this.memLifecycle);
      if (this.memIncludeInternal) params.set('include_internal', 'true');
      this.parseMemoryTags(this.memTags).forEach(function(tag) {
        params.append('tags', tag);
      });
      var query = params.toString();
      return '/api/memory/agents/' + this.memAgentId + '/kv' + (query ? '?' + query : '');
    },

    selectMemoryAgent() {
      this.memCleanupError = '';
      this.memCleanupResult = null;
      this.memCleanupRanAt = '';
      this.loadKv();
    },

    resetMemoryFilters() {
      this.memSearch = '';
      this.memNamespace = '';
      this.memLifecycle = '';
      this.memTags = '';
      this.memIncludeInternal = false;
      this.loadKv();
    },

    memoryValueText(value) {
      if (typeof value === 'object') return JSON.stringify(value, null, 2);
      return String(value);
    },

    lifecycleBadgeClass(state) {
      if (state === 'active') return 'badge-success';
      if (state === 'stale') return 'badge-warn';
      if (state === 'expired') return 'badge-error';
      return 'badge-dim';
    },

    formatTimestamp(value) {
      if (!value) return '-';
      var date = new Date(value);
      if (Number.isNaN(date.getTime())) return value;
      return date.toLocaleString();
    },

    cleanupActionClass(action) {
      if (action === 'migrate_legacy_key') return 'badge-warn';
      if (action === 'delete_legacy_key' || action === 'delete_orphan_metadata') return 'badge-error';
      if (action === 'backfill_metadata') return 'badge-created';
      return 'badge-dim';
    },

    cleanupActionLabel(action) {
      return String(action || '')
        .split('_')
        .filter(Boolean)
        .map(function(part) { return part.charAt(0).toUpperCase() + part.slice(1); })
        .join(' ');
    },

    async loadKv() {
      if (!this.memAgentId) {
        this.kvPairs = [];
        this.memCleanupError = '';
        this.memCleanupResult = null;
        this.memCleanupRanAt = '';
        return;
      }
      this.memLoading = true;
      this.memLoadError = '';
      try {
        var data = await OpenFangAPI.get(this.buildMemoryQuery());
        this.kvPairs = data.kv_pairs || [];
      } catch(e) {
        this.kvPairs = [];
        this.memLoadError = e.message || 'Could not load memory data.';
      }
      this.memLoading = false;
    },

    async runMemoryCleanup(apply) {
      if (!this.memAgentId || this.memCleanupLoading) return;
      this.memCleanupLoading = true;
      this.memCleanupError = '';
      try {
        var limit = Number(this.memCleanupLimit);
        if (!Number.isFinite(limit) || limit <= 0) limit = 50;
        limit = Math.min(Math.floor(limit), 200);
        var result = await OpenFangAPI.post(
          '/api/memory/agents/' + this.memAgentId + '/kv/cleanup',
          { apply: apply, limit: limit }
        );
        this.memCleanupResult = result;
        this.memCleanupRanAt = new Date().toISOString();
        if (apply) {
          OpenFangToast.success('Memory cleanup applied');
          await this.loadKv();
        } else {
          OpenFangToast.success('Memory cleanup audit complete');
        }
      } catch(e) {
        this.memCleanupError = e.message || 'Could not run memory cleanup.';
        OpenFangToast.error('Memory cleanup failed: ' + this.memCleanupError);
      }
      this.memCleanupLoading = false;
    },

    applyMemoryCleanup() {
      var self = this;
      OpenFangToast.confirm(
        'Apply Memory Cleanup',
        'Apply governance cleanup actions for the shared memory store? Legacy keys, orphan sidecars, or missing metadata may be changed immediately.',
        async function() {
          await self.runMemoryCleanup(true);
        }
      );
    },

    async addKey() {
      if (!this.memAgentId || !this.newKey.trim()) return;
      var value;
      try { value = JSON.parse(this.newValue); } catch(e) { value = this.newValue; }
      try {
        var payload = this.buildMemoryPayload(value, this.newKind, this.newTags, this.newFreshness);
        await OpenFangAPI.put('/api/memory/agents/' + this.memAgentId + '/kv/' + encodeURIComponent(this.newKey), payload);
        this.showAdd = false;
        OpenFangToast.success('Key "' + this.newKey + '" saved');
        this.resetAddForm();
        await this.loadKv();
      } catch(e) {
        OpenFangToast.error('Failed to save key: ' + e.message);
      }
    },

    deleteKey(key) {
      var self = this;
      OpenFangToast.confirm('Delete Key', 'Delete key "' + key + '"? This cannot be undone.', async function() {
        try {
          await OpenFangAPI.del('/api/memory/agents/' + self.memAgentId + '/kv/' + encodeURIComponent(key));
          OpenFangToast.success('Key "' + key + '" deleted');
          await self.loadKv();
        } catch(e) {
          OpenFangToast.error('Failed to delete key: ' + e.message);
        }
      });
    },

    startEdit(kv) {
      this.editingKey = kv.key;
      this.editingValue = this.memoryValueText(kv.value);
      this.editingKind = kv.kind || '';
      this.editingTags = (kv.tags || []).join(', ');
      this.editingFreshness = kv.freshness || '';
    },

    cancelEdit() {
      this.editingKey = null;
      this.editingValue = '';
      this.editingKind = '';
      this.editingTags = '';
      this.editingFreshness = '';
    },

    async saveEdit() {
      if (!this.editingKey || !this.memAgentId) return;
      var value;
      try { value = JSON.parse(this.editingValue); } catch(e) { value = this.editingValue; }
      try {
        var payload = this.buildMemoryPayload(value, this.editingKind, this.editingTags, this.editingFreshness);
        await OpenFangAPI.put('/api/memory/agents/' + this.memAgentId + '/kv/' + encodeURIComponent(this.editingKey), payload);
        OpenFangToast.success('Key "' + this.editingKey + '" updated');
        this.cancelEdit();
        await this.loadKv();
      } catch(e) {
        OpenFangToast.error('Failed to save: ' + e.message);
      }
    },

    get availableNamespaces() {
      var seen = {};
      return this.kvPairs
        .map(function(kv) { return kv.namespace || ''; })
        .filter(function(namespace) {
          if (!namespace || seen[namespace]) return false;
          seen[namespace] = true;
          return true;
        })
        .sort();
    },

    get filteredKvPairs() {
      var search = this.memSearch.trim().toLowerCase();
      if (!search) return this.kvPairs;
      return this.kvPairs.filter(function(kv) {
        return [
          kv.key,
          kv.namespace,
          kv.kind,
          kv.freshness,
          kv.source,
          kv.lifecycle_state,
          (kv.tags || []).join(' '),
          typeof kv.value === 'object' ? JSON.stringify(kv.value) : String(kv.value)
        ]
          .filter(Boolean)
          .join(' ')
          .toLowerCase()
          .indexOf(search) !== -1;
      });
    },

    get memoryStats() {
      return this.kvPairs.reduce(function(stats, kv) {
        stats.total += 1;
        if (kv.governed) stats.governed += 1;
        if (kv.internal) stats.internal += 1;
        if (kv.promotion_candidate) stats.promotion += 1;
        if (kv.lifecycle_state === 'active') stats.active += 1;
        if (kv.lifecycle_state === 'stale') stats.stale += 1;
        if (kv.lifecycle_state === 'expired') stats.expired += 1;
        return stats;
      }, {
        total: 0,
        governed: 0,
        internal: 0,
        promotion: 0,
        active: 0,
        stale: 0,
        expired: 0
      });
    },

    get cleanupSummaryCards() {
      if (!this.memCleanupResult || !this.memCleanupResult.summary) return [];
      var summary = this.memCleanupResult.summary;
      return [
        {
          label: 'Findings',
          value: summary.findings || 0,
          className: summary.findings ? 'stat-value-warning' : 'stat-value-success'
        },
        {
          label: 'Migrate Legacy',
          value: summary.migrate_legacy_key || 0,
          className: summary.migrate_legacy_key ? 'stat-value-warning' : ''
        },
        {
          label: 'Orphan Metadata',
          value: summary.delete_orphan_metadata || 0,
          className: summary.delete_orphan_metadata ? 'stat-value-accent' : ''
        },
        {
          label: 'Backfill Metadata',
          value: summary.backfill_metadata || 0,
          className: summary.backfill_metadata ? 'stat-value-accent' : ''
        }
      ];
    },

    get cleanupAppliedSummary() {
      if (!this.memCleanupResult || !this.memCleanupResult.apply || !this.memCleanupResult.summary) {
        return '';
      }
      var summary = this.memCleanupResult.summary;
      var parts = [];
      if (summary.applied_migrate_legacy_key) parts.push(summary.applied_migrate_legacy_key + ' migrated');
      if (summary.applied_delete_legacy_key) parts.push(summary.applied_delete_legacy_key + ' duplicate legacy deleted');
      if (summary.applied_delete_orphan_metadata) parts.push(summary.applied_delete_orphan_metadata + ' orphan sidecar deleted');
      if (summary.applied_backfill_metadata) parts.push(summary.applied_backfill_metadata + ' metadata backfilled');
      return parts.join(', ');
    }
  };
}
