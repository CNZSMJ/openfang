// OpenFang Agents Page — Multi-step spawn wizard, detail view with tabs, file editor, personality presets
'use strict';

function buildAgentSoulMd(agentName, archetype, soulStyle) {
  var role = (archetype || 'Assistant').toLowerCase();
  var soulText = soulStyle && soulStyle.trim()
    ? soulStyle.trim()
    : 'Be helpful, grounded, and direct. Adapt to the user and the task without becoming generic.';
  return [
    '# Soul',
    'You are ' + agentName + ', a ' + role + ' agent in OpenFang.',
    '',
    '## Core Character',
    soulText,
    '- Stay useful under ambiguity: ask for missing context only when it materially changes the outcome.',
    '- Keep a consistent voice, but let the task and user needs shape the level of depth.',
    '- Prefer momentum, clarity, and completed work over performative explanation.',
    '',
    '## Collaboration Style',
    '- Balance warmth with directness.',
    '- Adjust tone to the user without losing your role identity.',
    '- When a better specialist is needed, say so plainly and help route the task.'
  ].join('\n');
}

function buildAgentIdentityMd(agentName, archetype, vibe, color, emoji) {
  var role = (archetype || 'Assistant').toLowerCase();
  return [
    '---',
    'name: ' + agentName,
    'archetype: ' + role,
    'vibe: ' + (vibe || 'custom'),
    'emoji: ' + (emoji || ''),
    'avatar_url:',
    'greeting_style: warm',
    'color: ' + (color || ''),
    '---',
    '# Identity',
    '',
    '- Role: ' + role,
    '- Default vibe: ' + (vibe || 'custom'),
    '- Product stance: produce useful work first, then explain it clearly.',
    '- Visual identity fields above are user-editable anchors, not rigid rules.',
    '',
    '<!-- Visual identity and personality at a glance. Edit these fields freely. -->'
  ].join('\n');
}

function buildAgentBehaviorMd(archetype) {
  var role = archetype ? archetype.toLowerCase() : 'general-purpose assistant';
  var domainFocus = archetype
    ? 'Keep your approach consistent with a ' + role + ' role when choosing depth, terminology, and output shape.'
    : 'Keep your approach consistent with the agent\'s intended role.';
  return [
    '# Agent Behavioral Guidelines',
    '',
    '## Core Principles',
    '- Act first, narrate second. Prefer producing the work over only describing it.',
    '- Ask one clarifying question when necessary, not a long interrogation.',
    '- Preserve user momentum by being concrete, structured, and useful.',
    '- Store durable user context in memory when it will improve future turns.',
    '- Separate established facts from assumptions and recommendations.',
    '',
    '## Working Loop',
    '- Understand the real outcome the user needs before choosing the response shape.',
    '- Break multi-part tasks into visible chunks when it improves execution or reviewability.',
    '- Verify important details, edits, and calculations before presenting them as done.',
    '',
    '## Response Style',
    '- Lead with the result, recommendation, or answer.',
    '- Keep responses concise unless the user asks for depth.',
    '- Use formatting only when it improves readability.',
    '- Be explicit about uncertainty instead of bluffing.',
    '',
    '## Domain Focus',
    '- ' + domainFocus,
    '',
    '## Delegation & Boundaries',
    '- Handle work directly when you have enough context and capability to do it well.',
    '- If a specialist or different workflow would clearly outperform you, say so and help the user transition.'
  ].join('\n');
}

function buildAgentBootstrapMd(agentName, archetype) {
  var role = (archetype || 'general-purpose assistant').toLowerCase();
  return [
    '# First-Run Bootstrap',
    '',
    'On your first conversation with a new user:',
    '1. Introduce yourself as ' + agentName + ' with a one-line summary of your role.',
    '2. Ask for the user\'s name and one useful preference.',
    '3. Save that information in memory for future continuity.',
    '4. Briefly explain how you can help as a ' + role + '.',
    '5. If the user already included a request, start helping immediately.',
    '',
    '## First-Turn Priorities',
    '- Build trust through competence, clarity, and tone.',
    '- Avoid long autobiographies or capability dumps.',
    '- Capture durable user context, not transient noise.'
  ].join('\n');
}

function buildAgentUserMd(archetype) {
  var role = (archetype || 'this agent').toLowerCase();
  return [
    '# User',
    '<!-- Updated as the agent learns about the user -->',
    '',
    '## Profile',
    '- Name:',
    '- Timezone:',
    '- Communication style:',
    '',
    '## Preferences',
    '- Preferred level of detail:',
    '- Preferred output format:',
    '- Constraints or non-goals:',
    '',
    '## Ongoing Context',
    '- Active projects where this ' + role + ' agent is helping:',
    '- Standing requests or recurring workflows:',
    '- Follow-up items worth remembering:'
  ].join('\n');
}

function buildAgentToolsMd(archetype) {
  var role = (archetype || 'this').toLowerCase();
  return [
    '# Tools & Environment',
    '<!-- Project-specific environment notes and conventions -->',
    '',
    '## Tooling Notes',
    '- Record preferred commands, external systems, and repo conventions here.',
    '- Capture constraints that change how this agent should operate in practice.',
    '',
    '## Role Fit',
    '- This ' + role + ' agent should use tools to deliver concrete work, not to narrate intentions.',
    '- Use memory for durable context, files for persistent outputs, and external lookups only when they add value.'
  ].join('\n');
}

function buildAgentMemoryMd(archetype) {
  var role = (archetype || 'this agent').toLowerCase();
  return [
    '# Long-Term Memory',
    '<!-- Curated knowledge the agent preserves across sessions -->',
    '',
    '## Durable User Context',
    '- Preferences worth reusing:',
    '- Repeated patterns or workflows:',
    '',
    '## Stable Reference Facts',
    '- Long-lived facts relevant to ' + role + ' work:',
    '- Decisions or conventions that should persist:',
    '',
    '## Relationship Continuity',
    '- What would help this agent be more useful next time:'
  ].join('\n');
}

/** Escape a string for use inside TOML triple-quoted strings ("""\n...\n""").
 *  Backslashes are escaped, and runs of 3+ consecutive double-quotes are
 *  broken up so the TOML parser never sees an unintended closing delimiter.
 */
function tomlMultilineEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"""/g, '""\\"');
}

/** Escape a string for use inside a TOML basic (single-line) string ("...").
 *  Backslashes, double-quotes, and common control chars are escaped.
 */
function tomlBasicEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\r/g, '\\r').replace(/\t/g, '\\t');
}

function agentsPage() {
  return {
    tab: 'agents',
    activeChatAgent: null,
    // -- Agents state --
    showSpawnModal: false,
    showDetailModal: false,
    detailAgent: null,
    spawnMode: 'wizard',
    spawning: false,
    spawnToml: '',
    filterState: 'all',
    loading: true,
    loadError: '',
    spawnForm: {
      name: '',
      provider: 'groq',
      model: 'llama-3.3-70b-versatile',
      systemPrompt: 'You are an AI agent in OpenFang.',
      profile: 'full',
      caps: { memory_read: true, memory_write: true, network: false, shell: false, agent_spawn: false }
    },

    // -- Multi-step wizard state --
    spawnStep: 1,
    spawnIdentity: { emoji: '', color: '#FF5C00', archetype: '' },
    selectedPreset: '',
    soulContent: '',
    emojiOptions: [
      '\u{1F916}', '\u{1F4BB}', '\u{1F50D}', '\u{270D}\uFE0F', '\u{1F4CA}', '\u{1F6E0}\uFE0F',
      '\u{1F4AC}', '\u{1F393}', '\u{1F310}', '\u{1F512}', '\u{26A1}', '\u{1F680}',
      '\u{1F9EA}', '\u{1F3AF}', '\u{1F4D6}', '\u{1F9D1}\u200D\u{1F4BB}', '\u{1F4E7}', '\u{1F3E2}',
      '\u{2764}\uFE0F', '\u{1F31F}', '\u{1F527}', '\u{1F4DD}', '\u{1F4A1}', '\u{1F3A8}'
    ],
    archetypeOptions: ['Assistant', 'Researcher', 'Coder', 'Writer', 'DevOps', 'Support', 'Analyst', 'Custom'],
    personalityPresets: [
      { id: 'professional', label: 'Professional', soul: 'Communicate in a clear, professional tone. Be direct and structured. Use formal language and data-driven reasoning. Prioritize accuracy over personality.' },
      { id: 'friendly', label: 'Friendly', soul: 'Be warm, approachable, and conversational. Use casual language and show genuine interest in the user. Add personality to your responses while staying helpful.' },
      { id: 'technical', label: 'Technical', soul: 'Focus on technical accuracy and depth. Use precise terminology. Show your work and reasoning. Prefer code examples and structured explanations.' },
      { id: 'creative', label: 'Creative', soul: 'Be imaginative and expressive. Use vivid language, analogies, and unexpected connections. Encourage creative thinking and explore multiple perspectives.' },
      { id: 'concise', label: 'Concise', soul: 'Be extremely brief and to the point. No filler, no pleasantries. Answer in the fewest words possible while remaining accurate and complete.' },
      { id: 'mentor', label: 'Mentor', soul: 'Be patient and encouraging like a great teacher. Break down complex topics step by step. Ask guiding questions. Celebrate progress and build confidence.' }
    ],

    // -- Detail modal tabs --
    detailTab: 'info',
    agentFiles: [],
    editingFile: null,
    fileContent: '',
    fileSaving: false,
    filesLoading: false,
    memoryAutoconvergeLimit: 24,
    memoryAutoconvergeLoading: false,
    memoryAutoconvergeApplying: false,
    memoryAutoconvergeError: '',
    memoryAutoconvergePlan: null,
    memoryAutoconvergeRanAt: '',
    memoryAutoconvergePreviewing: false,
    configForm: {},
    configSaving: false,
    // -- Tool filters --
    toolFilters: { tool_allowlist: [], tool_blocklist: [] },
    toolFiltersLoading: false,
    newAllowTool: '',
    newBlockTool: '',
    // -- Model switch --
    editingModel: false,
    newModelValue: '',
    modelSaving: false,
    // -- Fallback chain --
    editingFallback: false,
    newFallbackValue: '',

    // -- Templates state --
    tplTemplates: [],
    tplProviders: [],
    tplLoading: false,
    tplLoadError: '',
    selectedCategory: 'All',
    searchQuery: '',

    builtinTemplates: [
      {
        name: 'General Assistant',
        description: 'A versatile conversational agent that can help with everyday tasks, answer questions, and provide recommendations.',
        category: 'General',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'full',
        system_prompt: 'You are a helpful, friendly assistant. Provide clear, accurate, and concise responses. Ask clarifying questions when needed.'
      },
      {
        name: 'Code Helper',
        description: 'A programming-focused agent that writes, reviews, and debugs code across multiple languages.',
        category: 'Development',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'coding',
        system_prompt: 'You are an expert programmer. Help users write clean, efficient code. Explain your reasoning. Follow best practices and conventions for the language being used.'
      },
      {
        name: 'Researcher',
        description: 'An analytical agent that breaks down complex topics, synthesizes information, and provides cited summaries.',
        category: 'Research',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'research',
        system_prompt: 'You are a research analyst. Break down complex topics into clear explanations. Provide structured analysis with key findings. Cite sources when available.'
      },
      {
        name: 'Writer',
        description: 'A creative writing agent that helps with drafting, editing, and improving written content of all kinds.',
        category: 'Writing',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'full',
        system_prompt: 'You are a skilled writer and editor. Help users create polished content. Adapt your tone and style to match the intended audience. Offer constructive suggestions for improvement.'
      },
      {
        name: 'Data Analyst',
        description: 'A data-focused agent that helps analyze datasets, create queries, and interpret statistical results.',
        category: 'Development',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'coding',
        system_prompt: 'You are a data analysis expert. Help users understand their data, write SQL/Python queries, and interpret results. Present findings clearly with actionable insights.'
      },
      {
        name: 'DevOps Engineer',
        description: 'A systems-focused agent for CI/CD, infrastructure, Docker, and deployment troubleshooting.',
        category: 'Development',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'automation',
        system_prompt: 'You are a DevOps engineer. Help with CI/CD pipelines, Docker, Kubernetes, infrastructure as code, and deployment. Prioritize reliability and security.'
      },
      {
        name: 'Customer Support',
        description: 'A professional, empathetic agent for handling customer inquiries and resolving issues.',
        category: 'Business',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'messaging',
        system_prompt: 'You are a professional customer support representative. Be empathetic, patient, and solution-oriented. Acknowledge concerns before offering solutions. Escalate complex issues appropriately.'
      },
      {
        name: 'Tutor',
        description: 'A patient educational agent that explains concepts step-by-step and adapts to the learner\'s level.',
        category: 'General',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'full',
        system_prompt: 'You are a patient and encouraging tutor. Explain concepts step by step, starting from fundamentals. Use analogies and examples. Check understanding before moving on. Adapt to the learner\'s pace.'
      },
      {
        name: 'API Designer',
        description: 'An agent specialized in RESTful API design, OpenAPI specs, and integration architecture.',
        category: 'Development',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'coding',
        system_prompt: 'You are an API design expert. Help users design clean, consistent RESTful APIs following best practices. Cover endpoint naming, request/response schemas, error handling, and versioning.'
      },
      {
        name: 'Meeting Notes',
        description: 'Summarizes meeting transcripts into structured notes with action items and key decisions.',
        category: 'Business',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'minimal',
        system_prompt: 'You are a meeting summarizer. When given a meeting transcript or notes, produce a structured summary with: key decisions, action items (with owners), discussion highlights, and follow-up questions.'
      }
    ],

    // ── Profile Descriptions ──
    profileDescriptions: {
      minimal: { label: 'Minimal', desc: 'Read-only file access' },
      coding: { label: 'Coding', desc: 'Files + shell + web fetch' },
      research: { label: 'Research', desc: 'Web search + file read/write' },
      messaging: { label: 'Messaging', desc: 'Agents + memory access' },
      automation: { label: 'Automation', desc: 'All tools except custom' },
      balanced: { label: 'Balanced', desc: 'General-purpose tool set' },
      precise: { label: 'Precise', desc: 'Focused tool set for accuracy' },
      creative: { label: 'Creative', desc: 'Full tools with creative emphasis' },
      full: { label: 'Full', desc: 'All 35+ tools' }
    },
    profileInfo: function(name) {
      return this.profileDescriptions[name] || { label: name, desc: '' };
    },

    // ── Tool Preview in Spawn Modal ──
    spawnProfiles: [],
    spawnProfilesLoaded: false,
    async loadSpawnProfiles() {
      if (this.spawnProfilesLoaded) return;
      try {
        var data = await OpenFangAPI.get('/api/profiles');
        this.spawnProfiles = data.profiles || [];
        this.spawnProfilesLoaded = true;
      } catch(e) { this.spawnProfiles = []; }
    },
    get selectedProfileTools() {
      var pname = this.spawnForm.profile;
      var match = this.spawnProfiles.find(function(p) { return p.name === pname; });
      if (match && match.tools) return match.tools.slice(0, 15);
      return [];
    },

    get agents() { return Alpine.store('app').agents; },

    get filteredAgents() {
      var f = this.filterState;
      if (f === 'all') return this.agents;
      return this.agents.filter(function(a) { return a.state.toLowerCase() === f; });
    },

    get runningCount() {
      return this.agents.filter(function(a) { return a.state === 'Running'; }).length;
    },

    get stoppedCount() {
      return this.agents.filter(function(a) { return a.state !== 'Running'; }).length;
    },

    // -- Templates computed --
    get categories() {
      var cats = { 'All': true };
      this.builtinTemplates.forEach(function(t) { cats[t.category] = true; });
      this.tplTemplates.forEach(function(t) { if (t.category) cats[t.category] = true; });
      return Object.keys(cats);
    },

    get filteredBuiltins() {
      var self = this;
      return this.builtinTemplates.filter(function(t) {
        if (self.selectedCategory !== 'All' && t.category !== self.selectedCategory) return false;
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          if (t.name.toLowerCase().indexOf(q) === -1 &&
              t.description.toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
    },

    get filteredCustom() {
      var self = this;
      return this.tplTemplates.filter(function(t) {
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          if ((t.name || '').toLowerCase().indexOf(q) === -1 &&
              (t.description || '').toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
    },

    isProviderConfigured(providerName) {
      if (!providerName) return false;
      var p = this.tplProviders.find(function(pr) { return pr.id === providerName; });
      return p ? p.auth_status === 'configured' : false;
    },

    async init() {
      var self = this;
      this.loading = true;
      this.loadError = '';
      try {
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        this.loadError = e.message || 'Could not load agents. Is the daemon running?';
      }
      this.loading = false;

      // If a pending agent was set (e.g. from wizard or redirect), open chat inline
      var store = Alpine.store('app');
      if (store.pendingAgent) {
        this.activeChatAgent = store.pendingAgent;
      }
      // Watch for future pendingAgent changes
      this.$watch('$store.app.pendingAgent', function(agent) {
        if (agent) {
          self.activeChatAgent = agent;
        }
      });
    },

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        this.loadError = e.message || 'Could not load agents.';
      }
      this.loading = false;
    },

    async loadTemplates() {
      this.tplLoading = true;
      this.tplLoadError = '';
      try {
        var results = await Promise.all([
          OpenFangAPI.get('/api/templates'),
          OpenFangAPI.get('/api/providers').catch(function() { return { providers: [] }; })
        ]);
        this.tplTemplates = results[0].templates || [];
        this.tplProviders = results[1].providers || [];
      } catch(e) {
        this.tplTemplates = [];
        this.tplLoadError = e.message || 'Could not load templates.';
      }
      this.tplLoading = false;
    },

    chatWithAgent(agent) {
      Alpine.store('app').pendingAgent = agent;
      this.activeChatAgent = agent;
    },

    closeChat() {
      this.activeChatAgent = null;
      OpenFangAPI.wsDisconnect();
    },

    async showDetail(agent) {
      this.detailAgent = agent;
      this.detailAgent._fallbacks = [];
      this.detailTab = 'info';
      this.agentFiles = [];
      this.editingFile = null;
      this.fileContent = '';
      this.memoryAutoconvergeError = '';
      this.memoryAutoconvergePlan = null;
      this.memoryAutoconvergeRanAt = '';
      this.memoryAutoconvergePreviewing = false;
      this.editingFallback = false;
      this.newFallbackValue = '';
      this.configForm = {
        name: agent.name || '',
        system_prompt: agent.system_prompt || '',
        emoji: (agent.identity && agent.identity.emoji) || '',
        color: (agent.identity && agent.identity.color) || '#FF5C00',
        archetype: (agent.identity && agent.identity.archetype) || '',
        vibe: (agent.identity && agent.identity.vibe) || ''
      };
      this.showDetailModal = true;
      // Fetch full agent detail to get fallback_models
      try {
        var full = await OpenFangAPI.get('/api/agents/' + agent.id);
        this.detailAgent._fallbacks = full.fallback_models || [];
      } catch(e) { /* ignore */ }
    },

    killAgent(agent) {
      var self = this;
      OpenFangToast.confirm('Stop Agent', 'Stop agent "' + agent.name + '"? The agent will be shut down.', async function() {
        try {
          await OpenFangAPI.del('/api/agents/' + agent.id);
          OpenFangToast.success('Agent "' + agent.name + '" stopped');
          self.showDetailModal = false;
          await Alpine.store('app').refreshAgents();
        } catch(e) {
          OpenFangToast.error('Failed to stop agent: ' + e.message);
        }
      });
    },

    killAllAgents() {
      var list = this.filteredAgents;
      if (!list.length) return;
      OpenFangToast.confirm('Stop All Agents', 'Stop ' + list.length + ' agent(s)? All agents will be shut down.', async function() {
        var errors = [];
        for (var i = 0; i < list.length; i++) {
          try {
            await OpenFangAPI.del('/api/agents/' + list[i].id);
          } catch(e) { errors.push(list[i].name + ': ' + e.message); }
        }
        await Alpine.store('app').refreshAgents();
        if (errors.length) {
          OpenFangToast.error('Some agents failed to stop: ' + errors.join(', '));
        } else {
          OpenFangToast.success(list.length + ' agent(s) stopped');
        }
      });
    },

    // ── Multi-step wizard navigation ──
    openSpawnWizard() {
      this.showSpawnModal = true;
      this.spawnStep = 1;
      this.spawnMode = 'wizard';
      this.spawnIdentity = { emoji: '', color: '#FF5C00', archetype: '' };
      this.selectedPreset = '';
      this.soulContent = '';
      this.spawnForm.name = '';
      this.spawnForm.systemPrompt = 'You are an AI agent in OpenFang.';
      this.spawnForm.profile = 'full';
    },

    nextStep() {
      if (this.spawnStep === 1 && !this.spawnForm.name.trim()) {
        OpenFangToast.warn('Please enter an agent name');
        return;
      }
      if (this.spawnStep < 5) this.spawnStep++;
    },

    prevStep() {
      if (this.spawnStep > 1) this.spawnStep--;
    },

    selectPreset(preset) {
      this.selectedPreset = preset.id;
      this.soulContent = preset.soul;
    },

    generateToml() {
      var f = this.spawnForm;
      var lines = [
        'name = "' + tomlBasicEscape(f.name) + '"',
        'module = "builtin:chat"'
      ];
      if (f.profile && f.profile !== 'custom') {
        lines.push('profile = "' + f.profile + '"');
      }
      lines.push('', '[model]');
      lines.push('provider = "' + f.provider + '"');
      lines.push('model = "' + f.model + '"');
      var systemPrompt = (f.systemPrompt && f.systemPrompt.trim())
        ? f.systemPrompt
        : ('You are ' + f.name + ', an AI agent in OpenFang.');
      lines.push('system_prompt = """\n' + tomlMultilineEscape(systemPrompt) + '\n"""');
      if (f.profile === 'custom') {
        lines.push('', '[capabilities]');
        if (f.caps.memory_read) lines.push('memory_read = ["*"]');
        if (f.caps.memory_write) lines.push('memory_write = ["self.*"]');
        if (f.caps.network) lines.push('network = ["*"]');
        if (f.caps.shell) lines.push('shell = ["*"]');
        if (f.caps.agent_spawn) lines.push('agent_spawn = true');
      }
      return lines.join('\n');
    },

    buildSpawnScaffold() {
      var f = this.spawnForm;
      var identity = this.spawnIdentity;
      var vibe = this.selectedPreset || 'custom';
      var archetype = identity.archetype || 'Assistant';
      var soulBody = this.soulContent.trim() || 'Be helpful, grounded, and direct. Adapt to the user and the task without becoming generic.';
      return {
        soul_md: buildAgentSoulMd(f.name, archetype, soulBody),
        agents_md: buildAgentBehaviorMd(archetype),
        identity_md: buildAgentIdentityMd(f.name, archetype, vibe, identity.color, identity.emoji),
        user_md: buildAgentUserMd(archetype),
        tools_md: buildAgentToolsMd(archetype),
        memory_md: buildAgentMemoryMd(archetype),
        bootstrap_md: buildAgentBootstrapMd(f.name, archetype)
      };
    },

    async setMode(agent, mode) {
      try {
        await OpenFangAPI.put('/api/agents/' + agent.id + '/mode', { mode: mode });
        agent.mode = mode;
        OpenFangToast.success('Mode set to ' + mode);
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        OpenFangToast.error('Failed to set mode: ' + e.message);
      }
    },

    async spawnAgent() {
      this.spawning = true;
      var toml = this.spawnMode === 'wizard' ? this.generateToml() : this.spawnToml;
      if (!toml.trim()) {
        this.spawning = false;
        OpenFangToast.warn('Manifest is empty \u2014 enter agent config first');
        return;
      }

      try {
        var payload = { manifest_toml: toml };
        if (this.spawnMode === 'wizard') {
          payload.scaffold = this.buildSpawnScaffold();
        }
        var res = await OpenFangAPI.post('/api/agents', payload);
        if (res.agent_id) {
          // Post-spawn: update identity metadata
          var patchBody = {};
          if (this.spawnIdentity.emoji) patchBody.emoji = this.spawnIdentity.emoji;
          if (this.spawnIdentity.color) patchBody.color = this.spawnIdentity.color;
          if (this.spawnIdentity.archetype) patchBody.archetype = this.spawnIdentity.archetype;
          if (this.selectedPreset) patchBody.vibe = this.selectedPreset;

          if (Object.keys(patchBody).length) {
            OpenFangAPI.patch('/api/agents/' + res.agent_id + '/config', patchBody).catch(function(e) { console.warn('Post-spawn config patch failed:', e.message); });
          }

          this.showSpawnModal = false;
          this.spawnForm.name = '';
          this.spawnToml = '';
          this.spawnStep = 1;
          OpenFangToast.success('Agent "' + (res.name || 'new') + '" spawned');
          await Alpine.store('app').refreshAgents();
          this.chatWithAgent({ id: res.agent_id, name: res.name, model_provider: '?', model_name: '?' });
        } else {
          OpenFangToast.error('Spawn failed: ' + (res.error || 'Unknown error'));
        }
      } catch(e) {
        OpenFangToast.error('Failed to spawn agent: ' + e.message);
      }
      this.spawning = false;
    },

    // ── Detail modal: Files tab ──
    async loadAgentFiles() {
      if (!this.detailAgent) return;
      this.filesLoading = true;
      try {
        var data = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id + '/files');
        this.agentFiles = data.files || [];
      } catch(e) {
        this.agentFiles = [];
        OpenFangToast.error('Failed to load files: ' + e.message);
      }
      this.filesLoading = false;
    },

    async loadMemoryAutoconvergePlan() {
      if (!this.detailAgent || this.memoryAutoconvergeLoading) return;
      this.memoryAutoconvergeLoading = true;
      this.memoryAutoconvergeError = '';
      try {
        var limit = Number(this.memoryAutoconvergeLimit);
        if (!Number.isFinite(limit) || limit <= 0) limit = 24;
        limit = Math.min(Math.floor(limit), 64);
        this.memoryAutoconvergeLimit = limit;
        var data = await OpenFangAPI.get(
          '/api/agents/' + this.detailAgent.id + '/memory/autoconverge?limit=' + limit
        );
        this.memoryAutoconvergePlan = data;
        this.memoryAutoconvergeRanAt = new Date().toISOString();
        if (this.memoryAutoconvergePreviewing && this.editingFile === 'MEMORY.md') {
          this.fileContent = data.proposed_content || '';
        }
      } catch(e) {
        this.memoryAutoconvergePlan = null;
        this.memoryAutoconvergeError = e.message || 'Could not audit MEMORY.md autoconverge.';
        OpenFangToast.error('Failed to audit MEMORY.md: ' + this.memoryAutoconvergeError);
      }
      this.memoryAutoconvergeLoading = false;
    },

    async openFile(file) {
      if (!file.exists) {
        // Create with empty content
        this.editingFile = file.name;
        this.fileContent = '';
        this.memoryAutoconvergePreviewing = false;
        return;
      }
      try {
        var data = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id + '/files/' + encodeURIComponent(file.name));
        this.editingFile = file.name;
        this.fileContent = data.content || '';
        this.memoryAutoconvergePreviewing = false;
      } catch(e) {
        OpenFangToast.error('Failed to read file: ' + e.message);
      }
    },

    openMemoryAutoconvergePreview() {
      if (!this.memoryAutoconvergePlan) return;
      this.editingFile = 'MEMORY.md';
      this.fileContent = this.memoryAutoconvergePlan.proposed_content || '';
      this.memoryAutoconvergePreviewing = true;
    },

    async runMemoryAutoconverge(apply) {
      if (!this.detailAgent) return;
      if (apply) {
        if (this.memoryAutoconvergeApplying) return;
        this.memoryAutoconvergeApplying = true;
      } else {
        if (this.memoryAutoconvergeLoading) return;
        this.memoryAutoconvergeLoading = true;
      }
      this.memoryAutoconvergeError = '';
      try {
        var limit = Number(this.memoryAutoconvergeLimit);
        if (!Number.isFinite(limit) || limit <= 0) limit = 24;
        limit = Math.min(Math.floor(limit), 64);
        this.memoryAutoconvergeLimit = limit;
        var result = await OpenFangAPI.post(
          '/api/agents/' + this.detailAgent.id + '/memory/autoconverge',
          { apply: apply, limit: limit }
        );
        this.memoryAutoconvergePlan = result;
        this.memoryAutoconvergeRanAt = new Date().toISOString();
        if (this.memoryAutoconvergePreviewing && this.editingFile === 'MEMORY.md') {
          this.fileContent = result.proposed_content || '';
          this.memoryAutoconvergePreviewing = !apply;
        }
        if (apply) {
          OpenFangToast.success('Managed MEMORY.md snapshot applied');
          await this.loadAgentFiles();
        } else {
          OpenFangToast.success('Managed MEMORY.md audit complete');
        }
      } catch(e) {
        this.memoryAutoconvergeError = e.message || 'Could not run MEMORY.md autoconverge.';
        OpenFangToast.error('MEMORY.md autoconverge failed: ' + this.memoryAutoconvergeError);
      }
      this.memoryAutoconvergeLoading = false;
      this.memoryAutoconvergeApplying = false;
    },

    applyMemoryAutoconverge() {
      var self = this;
      if (!this.memoryAutoconvergePlan || !this.memoryAutoconvergePlan.summary || !this.memoryAutoconvergePlan.summary.can_apply) return;
      OpenFangToast.confirm(
        'Apply MEMORY.md Snapshot',
        'Write the managed autoconverged snapshot into this agent workspace MEMORY.md now?',
        async function() {
          await self.runMemoryAutoconverge(true);
        }
      );
    },

    async saveFile() {
      if (!this.editingFile || !this.detailAgent) return;
      if (this.memoryAutoconvergePreviewing) {
        OpenFangToast.warn('This editor is showing a managed preview. Use Apply Managed Snapshot instead of saving manually.');
        return;
      }
      this.fileSaving = true;
      try {
        await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/files/' + encodeURIComponent(this.editingFile), { content: this.fileContent });
        OpenFangToast.success(this.editingFile + ' saved');
        await this.loadAgentFiles();
      } catch(e) {
        OpenFangToast.error('Failed to save file: ' + e.message);
      }
      this.fileSaving = false;
    },

    closeFileEditor() {
      this.editingFile = null;
      this.fileContent = '';
      this.memoryAutoconvergePreviewing = false;
    },

    // ── Detail modal: Config tab ──
    async saveConfig() {
      if (!this.detailAgent) return;
      this.configSaving = true;
      try {
        await OpenFangAPI.patch('/api/agents/' + this.detailAgent.id + '/config', this.configForm);
        OpenFangToast.success('Config updated');
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        OpenFangToast.error('Failed to save config: ' + e.message);
      }
      this.configSaving = false;
    },

    // ── Clone agent ──
    async cloneAgent(agent) {
      var newName = (agent.name || 'agent') + '-copy';
      try {
        var res = await OpenFangAPI.post('/api/agents/' + agent.id + '/clone', { new_name: newName });
        if (res.agent_id) {
          OpenFangToast.success('Cloned as "' + res.name + '"');
          await Alpine.store('app').refreshAgents();
          this.showDetailModal = false;
        }
      } catch(e) {
        OpenFangToast.error('Clone failed: ' + e.message);
      }
    },

    // -- Template methods --
    async spawnFromTemplate(name) {
      try {
        var data = await OpenFangAPI.get('/api/templates/' + encodeURIComponent(name));
        if (data.manifest_toml) {
          var res = await OpenFangAPI.post('/api/agents', { manifest_toml: data.manifest_toml });
          if (res.agent_id) {
            OpenFangToast.success('Agent "' + (res.name || name) + '" spawned from template');
            await Alpine.store('app').refreshAgents();
            this.chatWithAgent({ id: res.agent_id, name: res.name || name, model_provider: '?', model_name: '?' });
          }
        }
      } catch(e) {
        OpenFangToast.error('Failed to spawn from template: ' + e.message);
      }
    },

    // ── Clear agent history ──
    async clearHistory(agent) {
      var self = this;
      OpenFangToast.confirm('Clear History', 'Clear all conversation history for "' + agent.name + '"? This cannot be undone.', async function() {
        try {
          await OpenFangAPI.del('/api/agents/' + agent.id + '/history');
          OpenFangToast.success('History cleared for "' + agent.name + '"');
        } catch(e) {
          OpenFangToast.error('Failed to clear history: ' + e.message);
        }
      });
    },

    // ── Model switch ──
    async changeModel() {
      if (!this.detailAgent || !this.newModelValue.trim()) return;
      this.modelSaving = true;
      try {
        var resp = await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/model', { model: this.newModelValue.trim() });
        var providerInfo = (resp && resp.provider) ? ' (provider: ' + resp.provider + ')' : '';
        OpenFangToast.success('Model changed' + providerInfo + ' (memory reset)');
        this.editingModel = false;
        await Alpine.store('app').refreshAgents();
        // Refresh detailAgent
        var agents = Alpine.store('app').agents;
        for (var i = 0; i < agents.length; i++) {
          if (agents[i].id === this.detailAgent.id) { this.detailAgent = agents[i]; break; }
        }
      } catch(e) {
        OpenFangToast.error('Failed to change model: ' + e.message);
      }
      this.modelSaving = false;
    },

    // ── Fallback model chain ──
    async addFallback() {
      if (!this.detailAgent || !this.newFallbackValue.trim()) return;
      var parts = this.newFallbackValue.trim().split('/');
      var provider = parts.length > 1 ? parts[0] : this.detailAgent.model_provider;
      var model = parts.length > 1 ? parts.slice(1).join('/') : parts[0];
      if (!this.detailAgent._fallbacks) this.detailAgent._fallbacks = [];
      this.detailAgent._fallbacks.push({ provider: provider, model: model });
      try {
        await OpenFangAPI.patch('/api/agents/' + this.detailAgent.id + '/config', {
          fallback_models: this.detailAgent._fallbacks
        });
        OpenFangToast.success('Fallback added: ' + provider + '/' + model);
      } catch(e) {
        OpenFangToast.error('Failed to save fallbacks: ' + e.message);
        this.detailAgent._fallbacks.pop();
      }
      this.editingFallback = false;
      this.newFallbackValue = '';
    },

    async removeFallback(idx) {
      if (!this.detailAgent || !this.detailAgent._fallbacks) return;
      var removed = this.detailAgent._fallbacks.splice(idx, 1);
      try {
        await OpenFangAPI.patch('/api/agents/' + this.detailAgent.id + '/config', {
          fallback_models: this.detailAgent._fallbacks
        });
        OpenFangToast.success('Fallback removed');
      } catch(e) {
        OpenFangToast.error('Failed to save fallbacks: ' + e.message);
        this.detailAgent._fallbacks.splice(idx, 0, removed[0]);
      }
    },

    // ── Tool filters ──
    async loadToolFilters() {
      if (!this.detailAgent) return;
      this.toolFiltersLoading = true;
      try {
        this.toolFilters = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id + '/tools');
      } catch(e) {
        this.toolFilters = { tool_allowlist: [], tool_blocklist: [] };
      }
      this.toolFiltersLoading = false;
    },

    addAllowTool() {
      var t = this.newAllowTool.trim();
      if (t && this.toolFilters.tool_allowlist.indexOf(t) === -1) {
        this.toolFilters.tool_allowlist.push(t);
        this.newAllowTool = '';
        this.saveToolFilters();
      }
    },

    removeAllowTool(tool) {
      this.toolFilters.tool_allowlist = this.toolFilters.tool_allowlist.filter(function(t) { return t !== tool; });
      this.saveToolFilters();
    },

    addBlockTool() {
      var t = this.newBlockTool.trim();
      if (t && this.toolFilters.tool_blocklist.indexOf(t) === -1) {
        this.toolFilters.tool_blocklist.push(t);
        this.newBlockTool = '';
        this.saveToolFilters();
      }
    },

    removeBlockTool(tool) {
      this.toolFilters.tool_blocklist = this.toolFilters.tool_blocklist.filter(function(t) { return t !== tool; });
      this.saveToolFilters();
    },

    async saveToolFilters() {
      if (!this.detailAgent) return;
      try {
        await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/tools', this.toolFilters);
      } catch(e) {
        OpenFangToast.error('Failed to update tool filters: ' + e.message);
      }
    },

    async spawnBuiltin(t) {
      var toml = 'name = "' + tomlBasicEscape(t.name) + '"\n';
      toml += 'description = "' + tomlBasicEscape(t.description) + '"\n';
      toml += 'module = "builtin:chat"\n';
      toml += 'profile = "' + t.profile + '"\n\n';
      toml += '[model]\nprovider = "' + t.provider + '"\nmodel = "' + t.model + '"\n';
      toml += 'system_prompt = """\n' + tomlMultilineEscape(t.system_prompt) + '\n"""\n';

      try {
        var res = await OpenFangAPI.post('/api/agents', { manifest_toml: toml });
        if (res.agent_id) {
          OpenFangToast.success('Agent "' + t.name + '" spawned');
          await Alpine.store('app').refreshAgents();
          this.chatWithAgent({ id: res.agent_id, name: t.name, model_provider: t.provider, model_name: t.model });
        }
      } catch(e) {
        OpenFangToast.error('Failed to spawn agent: ' + e.message);
      }
    }
  };
}
