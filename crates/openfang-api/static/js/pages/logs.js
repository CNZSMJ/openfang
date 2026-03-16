// OpenFang Logs Page — Real-time log viewer (SSE streaming + polling fallback) + Audit Trail tab
'use strict';

function logsPage() {
  return {
    tab: 'live',
    // -- Live logs state --
    entries: [],
    levelFilter: '',
    textFilter: '',
    autoRefresh: true,
    hovering: false,
    loading: true,
    loadError: '',
    _pollTimer: null,

    // -- SSE streaming state --
    _eventSource: null,
    streamConnected: false,
    streamPaused: false,

    // -- Audit state --
    auditEntries: [],
    tipHash: '',
    chainValid: null,
    filterAction: '',
    auditLoading: false,
    auditLoadError: '',

    // -- Memory trace inspection state --
    memoryTraceAgentFilter: '',
    memoryTraceModeFilter: '',
    memoryTraceSourceFilter: '',
    memoryTraceTextFilter: '',
    expandedMemoryTraceSeqs: {},
    copiedMemoryTraceToken: '',
    _copiedMemoryTraceTimer: null,

    startStreaming: function() {
      var self = this;
      if (this._eventSource) { this._eventSource.close(); this._eventSource = null; }

      var url = '/api/logs/stream';
      var sep = '?';
      var token = OpenFangAPI.getToken();
      if (token) { url += sep + 'token=' + encodeURIComponent(token); sep = '&'; }

      try {
        this._eventSource = new EventSource(url);
      } catch(e) {
        // EventSource not supported or blocked; fall back to polling
        this.streamConnected = false;
        this.startPolling();
        return;
      }

      this._eventSource.onopen = function() {
        self.streamConnected = true;
        self.loading = false;
        self.loadError = '';
      };

      this._eventSource.onmessage = function(event) {
        if (self.streamPaused) return;
        try {
          var entry = JSON.parse(event.data);
          // Avoid duplicate entries by checking seq
          var dominated = false;
          for (var i = 0; i < self.entries.length; i++) {
            if (self.entries[i].seq === entry.seq) { dominated = true; break; }
          }
          if (!dominated) {
            self.entries.push(entry);
            // Cap at 500 entries (remove oldest)
            if (self.entries.length > 500) {
              self.entries.splice(0, self.entries.length - 500);
            }
            // Auto-scroll to bottom
            if (self.autoRefresh && !self.hovering) {
              self.$nextTick(function() {
                var el = document.getElementById('log-container');
                if (el) el.scrollTop = el.scrollHeight;
              });
            }
          }
        } catch(e) {
          // Ignore parse errors (heartbeat comments are not delivered to onmessage)
        }
      };

      this._eventSource.onerror = function() {
        self.streamConnected = false;
        if (self._eventSource) {
          self._eventSource.close();
          self._eventSource = null;
        }
        // Fall back to polling
        self.startPolling();
      };
    },

    startPolling: function() {
      var self = this;
      this.streamConnected = false;
      this.fetchLogs();
      if (this._pollTimer) clearInterval(this._pollTimer);
      this._pollTimer = setInterval(function() {
        if (self.autoRefresh && !self.hovering && (self.tab === 'live' || self.tab === 'memory') && !self.streamPaused) {
          self.fetchLogs();
        }
      }, 2000);
    },

    async fetchLogs() {
      if (this.loading) this.loadError = '';
      try {
        var data = await OpenFangAPI.get('/api/audit/recent?n=200');
        this.entries = data.entries || [];
        if (this.autoRefresh && !this.hovering) {
          this.$nextTick(function() {
            var el = document.getElementById('log-container');
            if (el) el.scrollTop = el.scrollHeight;
          });
        }
        if (this.loading) this.loading = false;
      } catch(e) {
        if (this.loading) {
          this.loadError = e.message || 'Could not load logs.';
          this.loading = false;
        }
      }
    },

    async loadData() {
      this.loading = true;
      return this.fetchLogs();
    },

    togglePause: function() {
      this.streamPaused = !this.streamPaused;
      if (!this.streamPaused && this.streamConnected) {
        // Resume: scroll to bottom
        var self = this;
        this.$nextTick(function() {
          var el = document.getElementById('log-container');
          if (el) el.scrollTop = el.scrollHeight;
        });
      }
    },

    clearLogs: function() {
      this.entries = [];
    },

    classifyLevel: function(action) {
      if (!action) return 'info';
      var a = action.toLowerCase();
      if (a.indexOf('error') !== -1 || a.indexOf('fail') !== -1 || a.indexOf('crash') !== -1) return 'error';
      if (a.indexOf('warn') !== -1 || a.indexOf('deny') !== -1 || a.indexOf('block') !== -1) return 'warn';
      return 'info';
    },

    isMemoryTrace: function(entry) {
      return !!(entry && entry.action === 'MemoryTrace' && entry.payload && typeof entry.payload === 'object');
    },

    memoryTracePayload: function(entry) {
      return this.isMemoryTrace(entry) ? entry.payload : null;
    },

    memoryTraceSummary: function(entry) {
      var payload = this.memoryTracePayload(entry);
      if (!payload) return [];
      return [
        String(payload.semantic_mode || 'unknown'),
        String(payload.semantic_candidates || 0) + ' semantic',
        String(payload.shared_candidates || 0) + ' shared',
        String(payload.maintenance_signals || 0) + ' maintenance',
        String(payload.attention_signals || 0) + ' attention',
        String(payload.session_summaries || 0) + ' session',
        String((payload.selected_fused_recall || []).length) + ' selected'
      ];
    },

    memoryTraceSelectedRecall: function(entry) {
      var payload = this.memoryTracePayload(entry);
      if (!payload || !Array.isArray(payload.selected_fused_recall)) return [];
      return payload.selected_fused_recall.slice(0, 3);
    },

    memoryTraceOverflowCount: function(entry) {
      var payload = this.memoryTracePayload(entry);
      if (!payload || !Array.isArray(payload.selected_fused_recall)) return 0;
      return Math.max(payload.selected_fused_recall.length - 3, 0);
    },

    memoryTraceMeta: function(recall) {
      if (!recall) return '';
      var weight = typeof recall.source_weight === 'number' ? recall.source_weight.toFixed(3) : '?';
      var score = typeof recall.fused_score === 'number' ? recall.fused_score.toFixed(5) : '?';
      return '#' + recall.selected_rank + ' ' + recall.source + ' r' + recall.source_rank + ' · w=' + weight + ' · t=' + recall.tie_break_priority + ' · score=' + score;
    },

    previewText: function(value, limit) {
      if (!value) return '';
      var text = String(value);
      if (text.length <= limit) return text;
      return text.slice(0, Math.max(limit - 1, 1)) + '…';
    },

    auditOutcomePreview: function(entry) {
      if (this.isMemoryTrace(entry)) return '';
      return this.previewText(entry && entry.outcome ? entry.outcome : '', 160);
    },

    memoryTraceRecallSourceClass: function(recall) {
      if (!recall || !recall.source) return 'memory-trace-source-unknown';
      return recall.source === 'shared' ? 'memory-trace-source-shared' : 'memory-trace-source-semantic';
    },

    memoryTraceAgentOptions: function() {
      var seen = {};
      var options = [];
      this.entries.forEach(function(entry) {
        if (!entry || !entry.agent_id || !entry.payload || seen[entry.agent_id]) return;
        seen[entry.agent_id] = true;
        options.push({ id: entry.agent_id, label: entry.agent_id });
      });
      options.sort(function(a, b) {
        return a.label.localeCompare(b.label);
      });
      return options;
    },

    memoryTraceMatchesFilters: function(entry) {
      if (!this.isMemoryTrace(entry)) return false;
      var payload = entry.payload || {};
      if (this.memoryTraceAgentFilter && entry.agent_id !== this.memoryTraceAgentFilter) return false;
      if (this.memoryTraceModeFilter && payload.semantic_mode !== this.memoryTraceModeFilter) return false;
      if (this.memoryTraceSourceFilter) {
        var recalls = Array.isArray(payload.selected_fused_recall) ? payload.selected_fused_recall : [];
        var hasSource = recalls.some(function(recall) { return recall && recall.source === this.memoryTraceSourceFilter; }.bind(this));
        if (!hasSource) return false;
      }
      if (this.memoryTraceTextFilter) {
        var haystack = (
          (entry.detail || '') + ' ' +
          (entry.agent_id || '') + ' ' +
          JSON.stringify(payload || {})
        ).toLowerCase();
        if (haystack.indexOf(this.memoryTraceTextFilter.toLowerCase()) === -1) return false;
      }
      return true;
    },

    get filteredMemoryTraceEntries() {
      var self = this;
      return this.entries
        .filter(function(entry) { return self.memoryTraceMatchesFilters(entry); })
        .slice()
        .reverse();
    },

    get memoryTraceCountLabel() {
      return this.filteredMemoryTraceEntries.length + ' of ' + this.entries.filter(this.isMemoryTrace).length + ' traces';
    },

    isMemoryTraceExpanded: function(entry) {
      return !!(entry && this.expandedMemoryTraceSeqs[entry.seq]);
    },

    toggleMemoryTraceExpanded: function(entry) {
      if (!entry) return;
      if (this.expandedMemoryTraceSeqs[entry.seq]) {
        delete this.expandedMemoryTraceSeqs[entry.seq];
      } else {
        this.expandedMemoryTraceSeqs[entry.seq] = true;
      }
      this.expandedMemoryTraceSeqs = Object.assign({}, this.expandedMemoryTraceSeqs);
    },

    memoryTraceVisibleRecall: function(entry) {
      var payload = this.memoryTracePayload(entry);
      var recalls = payload && Array.isArray(payload.selected_fused_recall) ? payload.selected_fused_recall : [];
      return this.isMemoryTraceExpanded(entry) ? recalls : recalls.slice(0, 2);
    },

    memoryTraceHiddenRecallCount: function(entry) {
      var payload = this.memoryTracePayload(entry);
      var recalls = payload && Array.isArray(payload.selected_fused_recall) ? payload.selected_fused_recall : [];
      return Math.max(recalls.length - this.memoryTraceVisibleRecall(entry).length, 0);
    },

    clearMemoryTraceFilters: function() {
      this.memoryTraceAgentFilter = '';
      this.memoryTraceModeFilter = '';
      this.memoryTraceSourceFilter = '';
      this.memoryTraceTextFilter = '';
    },

    copyMemoryTraceText: async function(text, token) {
      if (!text) return;
      try {
        await navigator.clipboard.writeText(text);
        this.copiedMemoryTraceToken = token;
        if (this._copiedMemoryTraceTimer) clearTimeout(this._copiedMemoryTraceTimer);
        this._copiedMemoryTraceTimer = setTimeout(function() {
          this.copiedMemoryTraceToken = '';
        }.bind(this), 1800);
        OpenFangToast.success('Copied to clipboard');
      } catch(e) {
        OpenFangToast.error('Copy failed');
      }
    },

    copyMemoryTracePayload: function(entry) {
      if (!this.isMemoryTrace(entry)) return;
      var payload = JSON.stringify(entry.payload || {}, null, 2);
      return this.copyMemoryTraceText(payload, 'payload-' + entry.seq);
    },

    copyMemoryTraceRecall: function(entry, recall) {
      if (!recall) return;
      return this.copyMemoryTraceText(recall.rendered || '', 'recall-' + entry.seq + '-' + recall.selected_rank);
    },

    exportMemoryTraces: function() {
      var traces = this.filteredMemoryTraceEntries.map(function(entry) {
        return {
          seq: entry.seq,
          timestamp: entry.timestamp,
          agent_id: entry.agent_id,
          detail: entry.detail,
          payload: entry.payload
        };
      });
      var blob = new Blob([JSON.stringify(traces, null, 2)], { type: 'application/json' });
      var url = URL.createObjectURL(blob);
      var a = document.createElement('a');
      a.href = url;
      a.download = 'openfang-memory-traces-' + new Date().toISOString().slice(0, 10) + '.json';
      a.click();
      URL.revokeObjectURL(url);
    },

    get filteredEntries() {
      var self = this;
      var levelF = this.levelFilter;
      var textF = this.textFilter.toLowerCase();
      return this.entries.filter(function(e) {
        if (levelF && self.classifyLevel(e.action) !== levelF) return false;
        if (textF) {
          var haystack = ((e.action || '') + ' ' + (e.detail || '') + ' ' + (e.outcome || '') + ' ' + (e.agent_id || '') + ' ' + JSON.stringify(e.payload || '')).toLowerCase();
          if (haystack.indexOf(textF) === -1) return false;
        }
        return true;
      });
    },

    get connectionLabel() {
      if (this.streamPaused) return 'Paused';
      if (this.streamConnected) return 'Live';
      if (this._pollTimer) return 'Polling';
      return 'Disconnected';
    },

    get connectionClass() {
      if (this.streamPaused) return 'paused';
      if (this.streamConnected) return 'live';
      if (this._pollTimer) return 'polling';
      return 'disconnected';
    },

    exportLogs: function() {
      var lines = this.filteredEntries.map(function(e) {
        return new Date(e.timestamp).toISOString() + ' [' + e.action + '] ' + (e.detail || '');
      });
      var blob = new Blob([lines.join('\n')], { type: 'text/plain' });
      var url = URL.createObjectURL(blob);
      var a = document.createElement('a');
      a.href = url;
      a.download = 'openfang-logs-' + new Date().toISOString().slice(0, 10) + '.txt';
      a.click();
      URL.revokeObjectURL(url);
    },

    // -- Audit methods --
    get filteredAuditEntries() {
      var self = this;
      if (!self.filterAction) return self.auditEntries;
      return self.auditEntries.filter(function(e) { return e.action === self.filterAction; });
    },

    async loadAudit() {
      this.auditLoading = true;
      this.auditLoadError = '';
      try {
        var data = await OpenFangAPI.get('/api/audit/recent?n=200');
        this.auditEntries = data.entries || [];
        this.tipHash = data.tip_hash || '';
      } catch(e) {
        this.auditEntries = [];
        this.auditLoadError = e.message || 'Could not load audit log.';
      }
      this.auditLoading = false;
    },

    auditAgentName: function(agentId) {
      if (!agentId) return '-';
      var agents = Alpine.store('app').agents || [];
      var agent = agents.find(function(a) { return a.id === agentId; });
      return agent ? agent.name : agentId.substring(0, 8) + '...';
    },

    friendlyAction: function(action) {
      if (!action) return 'Unknown';
      var map = {
        'AgentSpawn': 'Agent Created', 'AgentKill': 'Agent Stopped', 'AgentTerminated': 'Agent Stopped',
        'ToolInvoke': 'Tool Used', 'ToolResult': 'Tool Completed', 'AgentMessage': 'Message',
        'NetworkAccess': 'Network Access', 'ShellExec': 'Shell Command', 'FileAccess': 'File Access',
        'MemoryAccess': 'Memory Access', 'AuthAttempt': 'Login Attempt', 'AuthSuccess': 'Login Success',
        'AuthFailure': 'Login Failed', 'CapabilityDenied': 'Permission Denied', 'RateLimited': 'Rate Limited',
        'MemoryTrace': 'Memory Trace'
      };
      return map[action] || action.replace(/([A-Z])/g, ' $1').trim();
    },

    async verifyChain() {
      try {
        var data = await OpenFangAPI.get('/api/audit/verify');
        this.chainValid = data.valid === true;
        if (this.chainValid) {
          OpenFangToast.success('Audit chain verified — ' + (data.entries || 0) + ' entries valid');
        } else {
          OpenFangToast.error('Audit chain broken!');
        }
      } catch(e) {
        this.chainValid = false;
        OpenFangToast.error('Chain verification failed: ' + e.message);
      }
    },

    destroy: function() {
      if (this._eventSource) { this._eventSource.close(); this._eventSource = null; }
      if (this._pollTimer) { clearInterval(this._pollTimer); this._pollTimer = null; }
      if (this._copiedMemoryTraceTimer) { clearTimeout(this._copiedMemoryTraceTimer); this._copiedMemoryTraceTimer = null; }
    }
  };
}
