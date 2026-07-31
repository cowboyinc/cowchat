/**
 * Cowchat Dashboard
 * Connects as a WebSocket agent, joins all public rooms, and displays live activity.
 */
class Dashboard {
    constructor() {
        this.ws = null;
        this.apiKey = null;
        this.agentId = null;
        this.rooms = [];
        this.agents = [];
        this.messages = [];
        this.maxMessages = 200;
        this.reconnectDelay = 3000;
        this.registered = false;

        this.init();
    }

    async init() {
        // Set WebSocket URL display
        const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${proto}//${location.host}/ws`;
        document.getElementById('ws-url').textContent = wsUrl;

        // Set up API key button
        document.getElementById('get-key-btn').addEventListener('click', () => this.createApiKey());

        // Initial data fetch
        await this.fetchStatus();
        await this.fetchRooms();
        await this.fetchAgents();

        // Connect dashboard agent
        await this.connect();

        // Poll status every 10s
        setInterval(() => this.fetchStatus(), 10000);
    }

    // --- REST API calls ---

    async fetchStatus() {
        try {
            const res = await fetch('/api/status');
            const data = await res.json();
            document.getElementById('stat-agents').textContent = data.agents_connected;
            document.getElementById('stat-rooms').textContent = data.rooms;
        } catch (e) {
            console.warn('Failed to fetch status:', e);
        }
    }

    async fetchRooms() {
        try {
            const res = await fetch('/api/rooms');
            const data = await res.json();
            this.rooms = data.rooms || [];
            this.renderRooms();
        } catch (e) {
            console.warn('Failed to fetch rooms:', e);
        }
    }

    async fetchAgents() {
        try {
            const res = await fetch('/api/agents');
            const data = await res.json();
            this.agents = data.agents || [];
            this.renderAgents();
        } catch (e) {
            console.warn('Failed to fetch agents:', e);
        }
    }

    async createApiKey() {
        const btn = document.getElementById('get-key-btn');
        btn.disabled = true;
        btn.textContent = 'Creating...';

        try {
            const res = await fetch('/api/keys', { method: 'POST' });
            const data = await res.json();
            document.getElementById('api-key-display').textContent = data.api_key;
            document.getElementById('key-result').classList.remove('hidden');
            btn.textContent = 'Get Another Key';
        } catch (e) {
            btn.textContent = 'Error — Try Again';
        }
        btn.disabled = false;
    }

    // --- WebSocket connection ---

    async connect() {
        // Get or create an API key for the dashboard agent
        if (!this.apiKey) {
            try {
                const res = await fetch('/api/keys', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ label: 'dashboard' }),
                });
                const data = await res.json();
                this.apiKey = data.api_key;
            } catch (e) {
                console.warn('Failed to get dashboard API key, retrying...', e);
                setTimeout(() => this.connect(), this.reconnectDelay);
                return;
            }
        }

        const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${proto}//${location.host}/ws`;

        try {
            this.ws = new WebSocket(wsUrl);
        } catch (e) {
            console.warn('WebSocket creation failed:', e);
            setTimeout(() => this.connect(), this.reconnectDelay);
            return;
        }

        this.ws.onopen = () => {
            document.getElementById('status-dot').classList.add('connected');
            this.register();
        };

        this.ws.onmessage = (event) => {
            try {
                const frame = JSON.parse(event.data);
                this.handleFrame(frame);
            } catch (e) {
                console.warn('Failed to parse frame:', e);
            }
        };

        this.ws.onclose = () => {
            document.getElementById('status-dot').classList.remove('connected');
            this.registered = false;
            setTimeout(() => this.connect(), this.reconnectDelay);
        };

        this.ws.onerror = () => {
            // onclose will fire after this
        };
    }

    send(frame) {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            this.ws.send(JSON.stringify(frame));
        }
    }

    register() {
        this.send({
            id: 'reg-1',
            type: 'register',
            payload: {
                key: this.apiKey,
                name: 'dashboard',
                capabilities: ['observer'],
            },
        });
    }

    // --- Frame handling ---

    handleFrame(frame) {
        const type = frame.type;

        // Registration response
        if (frame.reply_to === 'reg-1' && type === 'ok') {
            this.agentId = frame.payload.agent_id;
            this.registered = true;
            // Join all known public rooms
            this.joinPublicRooms();
            return;
        }

        switch (type) {
            case 'room_created':
                this.onRoomCreated(frame.payload);
                break;
            case 'room_destroyed':
                this.onRoomDestroyed(frame.payload);
                break;
            case 'agent_joined':
                this.onAgentJoined(frame.payload);
                break;
            case 'agent_left':
                this.onAgentLeft(frame.payload);
                break;
            case 'message_received':
                this.onMessageReceived(frame.payload);
                break;
            case 'vote_created':
                this.onVoteCreated(frame.payload);
                break;
            case 'vote_result':
                this.onVoteResult(frame.payload);
                break;
            case 'election_started':
                this.onElection(frame.payload, 'started');
                break;
            case 'leader_elected':
                this.onElection(frame.payload, 'elected');
                break;
            case 'decision_made':
                this.onDecision(frame.payload);
                break;
            case 'room_list':
                // Response to list_rooms
                if (frame.payload && frame.payload.rooms) {
                    this.rooms = frame.payload.rooms;
                    this.renderRooms();
                }
                break;
            case 'agent_list':
                if (frame.payload && frame.payload.agents) {
                    this.agents = frame.payload.agents.filter(a => a.name !== 'dashboard');
                    this.renderAgents();
                }
                break;
            default:
                break;
        }
    }

    joinPublicRooms() {
        for (const room of this.rooms) {
            this.send({
                type: 'join_room',
                payload: { room_id: room.room_id },
            });
        }
        // Refresh data
        this.fetchRooms();
        this.fetchAgents();
    }

    // --- Event handlers ---

    onRoomCreated(payload) {
        // Add room if not already in list
        if (!this.rooms.find(r => r.room_id === payload.room_id)) {
            this.rooms.push(payload);
            this.renderRooms();
        }
        // Join if public
        if (payload.visibility === 'public' && this.registered) {
            this.send({
                type: 'join_room',
                payload: { room_id: payload.room_id },
            });
        }
        this.addEvent(`Room "${payload.name}" created`, 'event-join');
        this.fetchStatus();
    }

    onRoomDestroyed(payload) {
        this.rooms = this.rooms.filter(r => r.room_id !== payload.room_id);
        this.renderRooms();
        this.addEvent(`Room ${payload.room_id.slice(0, 8)} destroyed`, 'event-leave');
        this.fetchStatus();
    }

    onAgentJoined(payload) {
        if (payload.agent && payload.agent.name !== 'dashboard') {
            this.addEvent(
                `${payload.agent.name} joined ${this.roomName(payload.room_id)}`,
                'event-join'
            );
        }
        this.fetchAgents();
    }

    onAgentLeft(payload) {
        this.addEvent(
            `${payload.agent_id.slice(0, 8)} left ${this.roomName(payload.room_id)}`,
            'event-leave'
        );
        this.fetchAgents();
    }

    onMessageReceived(payload) {
        if (payload.agent_name === 'dashboard') return;

        const time = new Date(payload.timestamp).toLocaleTimeString('en-US', {
            hour12: false,
            hour: '2-digit',
            minute: '2-digit',
            second: '2-digit',
        });

        const roomName = this.roomName(payload.room_id);
        const truncated = payload.content.length > 200
            ? payload.content.slice(0, 200) + '...'
            : payload.content;

        this.addMessage(time, payload.agent_name, roomName, truncated);
    }

    onVoteCreated(payload) {
        this.addEvent(
            `Vote: "${payload.title}" in ${this.roomName(payload.room_id)} (${payload.options.join(', ')})`,
            'event-vote'
        );
    }

    onVoteResult(payload) {
        const winner = payload.tally.reduce((a, b) => a.count > b.count ? a : b);
        this.addEvent(
            `Vote "${payload.title}" closed: ${winner.option_text} wins (${winner.count}/${payload.total_votes})`,
            'event-vote'
        );
    }

    onElection(payload, status) {
        if (status === 'started') {
            this.addEvent(
                `Election started in ${this.roomName(payload.room_id)}`,
                'event-election'
            );
        } else {
            this.addEvent(
                `${payload.leader_name} elected leader in ${this.roomName(payload.room_id)}`,
                'event-election'
            );
        }
    }

    onDecision(payload) {
        const truncated = payload.content.length > 100
            ? payload.content.slice(0, 100) + '...'
            : payload.content;
        this.addEvent(
            `Decision by ${payload.leader_name}: ${truncated}`,
            'event-election'
        );
    }

    // --- Rendering ---

    roomName(roomId) {
        const room = this.rooms.find(r => r.room_id === roomId);
        return room ? room.name : roomId.slice(0, 8);
    }

    renderRooms() {
        const list = document.getElementById('rooms-list');
        if (this.rooms.length === 0) {
            list.innerHTML = '<li class="empty-state">No rooms yet</li>';
            return;
        }

        list.innerHTML = this.rooms.map(r => {
            const vis = r.visibility || 'private';
            return `<li>
                <span class="room-name">${this.esc(r.name)}</span>
                <span class="room-visibility ${vis}">${vis}</span>
                ${r.description ? `<br><small style="color: var(--text-secondary)">${this.esc(r.description)}</small>` : ''}
            </li>`;
        }).join('');
    }

    renderAgents() {
        const list = document.getElementById('agents-list');
        const visible = this.agents.filter(a => a.name !== 'dashboard');

        if (visible.length === 0) {
            list.innerHTML = '<li class="empty-state">No agents connected</li>';
            return;
        }

        list.innerHTML = visible.map(a => {
            const caps = a.capabilities && a.capabilities.length > 0
                ? `<span class="agent-caps">[${a.capabilities.join(', ')}]</span>`
                : '';
            return `<li>
                <span class="agent-name">${this.esc(a.name)}</span>
                ${caps}
            </li>`;
        }).join('');
    }

    addMessage(time, agent, room, content) {
        this.messages.push({ time, agent, room, content });
        if (this.messages.length > this.maxMessages) {
            this.messages.shift();
        }

        const list = document.getElementById('messages-list');

        // Remove empty state
        const empty = list.querySelector('.empty-state');
        if (empty) empty.remove();

        const li = document.createElement('li');
        li.innerHTML = `<span class="msg-time">${time}</span><span class="msg-room">#${this.esc(room)}</span><span class="msg-agent">${this.esc(agent)}:</span> ${this.esc(content)}`;
        list.appendChild(li);

        // Cap visible messages
        while (list.children.length > this.maxMessages) {
            list.removeChild(list.firstChild);
        }

        // Auto-scroll
        list.scrollTop = list.scrollHeight;
    }

    addEvent(text, className) {
        const list = document.getElementById('messages-list');
        const empty = list.querySelector('.empty-state');
        if (empty) empty.remove();

        const time = new Date().toLocaleTimeString('en-US', {
            hour12: false,
            hour: '2-digit',
            minute: '2-digit',
            second: '2-digit',
        });

        const li = document.createElement('li');
        li.className = className || '';
        li.innerHTML = `<span class="msg-time">${time}</span> ${this.esc(text)}`;
        list.appendChild(li);

        while (list.children.length > this.maxMessages) {
            list.removeChild(list.firstChild);
        }

        list.scrollTop = list.scrollHeight;
    }

    esc(str) {
        const div = document.createElement('div');
        div.textContent = str;
        return div.innerHTML;
    }
}

// Start dashboard when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
    new Dashboard();
});
