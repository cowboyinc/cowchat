import Darwin
import Foundation

enum LocalServerSupervisorError: LocalizedError {
    case missingBundledServer(URL)
    case bundledServerNotExecutable(URL)
    case launchFailed(String)
    case bundledServerExited(reason: String, diagnostic: String?)
    case bundledServerStopping

    var errorDescription: String? {
        switch self {
        case .missingBundledServer(let url):
            return "Cowchat's bundled local server is missing at \(url.path). Reinstall Cowchat."
        case .bundledServerNotExecutable(let url):
            return "Cowchat's bundled local server is not executable at \(url.path). Reinstall Cowchat."
        case .launchFailed(let message):
            return "Cowchat could not start its local server: \(message). Reinstall Cowchat if this continues."
        case .bundledServerExited(let reason, let diagnostic):
            let detail = diagnostic.map { " Server detail: \($0)" } ?? ""
            return "Cowchat's bundled local server exited \(reason).\(detail) Quit any other local Cowchat server and reconnect; reinstall Cowchat if this continues."
        case .bundledServerStopping:
            return "Cowchat's local server is still stopping. Reconnect in a moment."
        }
    }
}

@MainActor
protocol LocalServerSupervising: AnyObject {
    var ownsRunningServer: Bool { get }
    func launchIfNeeded() async throws
    func prepareForExplicitRetry()
    func stopOwnedServer()
    func shutdownOwnedServerForAppTermination() async
}

private struct CapturedStandardError: Sendable {
    let data: Data
    let wasTruncated: Bool
}

/// Drains stderr for the entire child lifetime so a noisy failed child cannot
/// block on a full pipe. Only the bounded prefix is retained in memory.
private final class BoundedStandardErrorCapture: @unchecked Sendable {
    let writer: FileHandle
    private let readerTask: Task<CapturedStandardError, Never>

    init(maxBytes: Int) {
        let pipe = Pipe()
        writer = pipe.fileHandleForWriting
        let reader = pipe.fileHandleForReading
        readerTask = Task.detached {
            var captured = Data()
            var wasTruncated = false

            while true {
                let chunk: Data
                do {
                    guard let next = try reader.read(upToCount: 8_192), !next.isEmpty else {
                        break
                    }
                    chunk = next
                } catch {
                    break
                }

                let remaining = max(0, maxBytes - captured.count)
                if remaining > 0 {
                    captured.append(chunk.prefix(remaining))
                }
                if chunk.count > remaining {
                    wasTruncated = true
                }
            }

            try? reader.close()
            return CapturedStandardError(data: captured, wasTruncated: wasTruncated)
        }
    }

    func closeParentWriter() {
        try? writer.close()
    }

    func diagnostic() async -> String? {
        let captured = await readerTask.value
        return Self.sanitizedDiagnostic(from: captured)
    }

    private static func sanitizedDiagnostic(from captured: CapturedStandardError) -> String? {
        var text = String(decoding: captured.data, as: UTF8.self)
        text.unicodeScalars.removeAll { scalar in
            (scalar.value < 0x20 || (0x7F ... 0x9F).contains(scalar.value))
                && scalar != "\n"
                && scalar != "\t"
        }
        text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }

        let redactions = [
            (#"(?i)\b(api[-_ ]?key|token|secret|password|authorization)\b[^\r\n:=]{0,80}[:=]\s*[^\s,;]+"#, "$1=[REDACTED]"),
            (#"(?i)\bbearer\s+[^\s,;]+"#, "Bearer [REDACTED]"),
            (#"(?i)(?<![a-z0-9])[a-z0-9+/_=-]{24,}(?![a-z0-9])"#, "[REDACTED]"),
        ]
        for (pattern, replacement) in redactions {
            guard let expression = try? NSRegularExpression(pattern: pattern) else { continue }
            let range = NSRange(text.startIndex..., in: text)
            text = expression.stringByReplacingMatches(
                in: text,
                range: range,
                withTemplate: replacement
            )
        }

        // A single compact line is useful in the UI and prevents terminal-like
        // control formatting from making a launch failure misleading.
        text = text
            .split(whereSeparator: \Character.isNewline)
            .joined(separator: " ")
            .replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
        if captured.wasTruncated {
            text += "…"
        }
        return text
    }
}

/// Owns one `posix_spawn` child and is the only code allowed to reap or signal it.
/// The lock makes the liveness check and signal atomic with respect to reaping:
/// if the child exits between `waitpid(WNOHANG)` and `kill`, it remains a zombie,
/// so its PID cannot be reused for an unrelated process.
private final class ExactChildProcess: @unchecked Sendable {
    let processIdentifier: pid_t

    private let lock = NSLock()
    private var rawWaitStatus: Int32?
    private var wasReapedElsewhere = false

    private init(processIdentifier: pid_t) {
        self.processIdentifier = processIdentifier
    }

    static func spawn(
        executableURL: URL,
        arguments: [String],
        standardInput: FileHandle,
        standardError: FileHandle
    ) throws -> ExactChildProcess {
        var actions: posix_spawn_file_actions_t?
        var attributes: posix_spawnattr_t?
        let nullOutput = Darwin.open("/dev/null", O_WRONLY)
        guard nullOutput >= 0 else { throw POSIXError(.EIO) }
        defer { Darwin.close(nullOutput) }

        try check(posix_spawn_file_actions_init(&actions))
        defer { posix_spawn_file_actions_destroy(&actions) }
        try check(posix_spawnattr_init(&attributes))
        defer { posix_spawnattr_destroy(&attributes) }

        let flags = Int16(POSIX_SPAWN_CLOEXEC_DEFAULT)
        try check(posix_spawnattr_setflags(&attributes, flags))
        try check(
            posix_spawn_file_actions_adddup2(
                &actions,
                standardInput.fileDescriptor,
                STDIN_FILENO
            )
        )
        try check(
            posix_spawn_file_actions_adddup2(
                &actions,
                nullOutput,
                STDOUT_FILENO
            )
        )
        try check(
            posix_spawn_file_actions_adddup2(
                &actions,
                standardError.fileDescriptor,
                STDERR_FILENO
            )
        )

        let strings = [executableURL.path] + arguments
        var argv: [UnsafeMutablePointer<CChar>?] = strings.map { strdup($0) }
        guard !argv.contains(where: { $0 == nil }) else {
            argv.forEach { free($0) }
            throw POSIXError(.ENOMEM)
        }
        defer { argv.forEach { free($0) } }
        argv.append(nil)

        var childPID: pid_t = 0
        let result = argv.withUnsafeMutableBufferPointer { buffer in
            posix_spawn(
                &childPID,
                executableURL.path,
                &actions,
                &attributes,
                buffer.baseAddress,
                environ
            )
        }
        try check(result)
        return ExactChildProcess(processIdentifier: childPID)
    }

    var isRunning: Bool {
        lock.withLock {
            pollForExitLocked()
            return rawWaitStatus == nil && !wasReapedElsewhere
        }
    }

    /// Signals only while this object still owns an unreaped child identity.
    @discardableResult
    func sendSignal(_ signal: Int32) -> Bool {
        lock.withLock {
            pollForExitLocked()
            guard rawWaitStatus == nil, !wasReapedElsewhere else { return true }

            var result: Int32
            repeat {
                result = Darwin.kill(processIdentifier, signal)
            } while result != 0 && errno == EINTR
            if result == 0 { return true }
            if errno == ESRCH {
                pollForExitLocked()
                return rawWaitStatus != nil || wasReapedElsewhere
            }
            return false
        }
    }

    func waitUntilExit() {
        while isRunning {
            usleep(10_000)
        }
    }

    func terminationReason() -> String {
        lock.withLock {
            pollForExitLocked()
            guard let rawWaitStatus else {
                return "before it was ready"
            }
            let terminatingSignal = rawWaitStatus & 0x7f
            if terminatingSignal == 0 {
                let exitStatus = (rawWaitStatus >> 8) & 0xff
                return "with status \(exitStatus) before it was ready"
            }
            return "from signal \(terminatingSignal) before it was ready"
        }
    }

    private func pollForExitLocked() {
        guard rawWaitStatus == nil, !wasReapedElsewhere else { return }

        var status: Int32 = 0
        let result = waitpid(processIdentifier, &status, WNOHANG)
        if result == processIdentifier {
            rawWaitStatus = status
        } else if result == -1, errno == ECHILD {
            // Fail closed: if another runtime component ever reaped this child,
            // never signal the numeric PID again.
            wasReapedElsewhere = true
        }
    }

    private static func check(_ result: Int32) throws {
        guard result == 0 else {
            throw POSIXError(POSIXErrorCode(rawValue: result) ?? .EIO)
        }
    }
}

@MainActor
final class LocalServerSupervisor: LocalServerSupervising {
    private enum AppControlCommand: String {
        case interrupt = "INT"
        case terminate = "TERM"
        case kill = "KILL"
    }

    private final class OwnedLaunch {
        let process: ExactChildProcess
        let standardErrorCapture: BoundedStandardErrorCapture
        private let controlWriter: FileHandle
        private var controlWriterIsClosed = false
        var stopRequested = false
        var interruptWasSent = false

        init(
            process: ExactChildProcess,
            standardErrorCapture: BoundedStandardErrorCapture,
            controlWriter: FileHandle
        ) {
            self.process = process
            self.standardErrorCapture = standardErrorCapture
            self.controlWriter = controlWriter

            // A child may exit between any status observation and a write.
            // Suppress SIGPIPE so that race becomes a closed exact channel,
            // never an app crash or a reason to fall back to signaling a PID.
            _ = Darwin.fcntl(controlWriter.fileDescriptor, F_SETNOSIGPIPE, 1)
        }

        func send(_ command: AppControlCommand) {
            guard !controlWriterIsClosed else { return }
            do {
                try controlWriter.write(contentsOf: Data("\(command.rawValue)\n".utf8))
            } catch {
                closeControlChannel()
            }
        }

        func closeControlChannel() {
            guard !controlWriterIsClosed else { return }
            controlWriterIsClosed = true
            try? controlWriter.close()
        }

        deinit {
            closeControlChannel()
        }
    }

    private let bundleURL: URL
    private let fileManager: FileManager
    private let startupGraceNanoseconds: UInt64
    private let maxStandardErrorBytes: Int
    private let interruptShutdownGraceNanoseconds: UInt64
    private let terminateShutdownGraceNanoseconds: UInt64
    private let controlKillShutdownGraceNanoseconds: UInt64
    private let shutdownPollIntervalNanoseconds: UInt64
    private var ownedLaunch: OwnedLaunch?
    private var retainedLaunchFailure: LocalServerSupervisorError?
    private var normalStopTask: Task<Void, Never>?
    private var applicationShutdownRequested = false
    private var applicationShutdownTask: Task<Void, Never>?

    init(
        bundleURL: URL = Bundle.main.bundleURL,
        fileManager: FileManager = .default,
        startupGraceNanoseconds: UInt64 = 250_000_000,
        maxStandardErrorBytes: Int = 2_048,
        interruptShutdownGraceNanoseconds: UInt64 = 1_000_000_000,
        terminateShutdownGraceNanoseconds: UInt64 = 1_000_000_000,
        controlKillShutdownGraceNanoseconds: UInt64 = 250_000_000,
        shutdownPollIntervalNanoseconds: UInt64 = 20_000_000
    ) {
        self.bundleURL = bundleURL
        self.fileManager = fileManager
        self.startupGraceNanoseconds = startupGraceNanoseconds
        self.maxStandardErrorBytes = max(0, maxStandardErrorBytes)
        self.interruptShutdownGraceNanoseconds = interruptShutdownGraceNanoseconds
        self.terminateShutdownGraceNanoseconds = terminateShutdownGraceNanoseconds
        self.controlKillShutdownGraceNanoseconds = controlKillShutdownGraceNanoseconds
        self.shutdownPollIntervalNanoseconds = max(1, shutdownPollIntervalNanoseconds)
    }

    var ownsRunningServer: Bool {
        ownedLaunch?.process.isRunning == true
    }

    func launchIfNeeded() async throws {
        guard !applicationShutdownRequested else {
            throw LocalServerSupervisorError.bundledServerStopping
        }

        if let launch = ownedLaunch {
            if launch.stopRequested {
                if let normalStopTask {
                    await normalStopTask.value
                }
                guard !launch.process.isRunning else {
                    throw LocalServerSupervisorError.bundledServerStopping
                }
                if ownedLaunch === launch {
                    launch.closeControlChannel()
                    ownedLaunch = nil
                }
            } else {
                if launch.process.isRunning { return }
                let failure = await failureForExitedProcess(
                    launch.process,
                    capture: launch.standardErrorCapture
                )
                if ownedLaunch === launch {
                    ownedLaunch = nil
                    retainedLaunchFailure = failure
                }
                throw failure
            }
        }
        // The normal-stop await above yields the main actor. App termination
        // may establish its permanent launch fence while this reconnect is
        // suspended, so recheck before creating any new exact child.
        guard !applicationShutdownRequested else {
            throw LocalServerSupervisorError.bundledServerStopping
        }
        if let retainedLaunchFailure { throw retainedLaunchFailure }

        let executableURL = Self.serverExecutableURL(in: bundleURL)
        guard fileManager.fileExists(atPath: executableURL.path) else {
            let failure = LocalServerSupervisorError.missingBundledServer(executableURL)
            retainedLaunchFailure = failure
            throw failure
        }
        guard fileManager.isExecutableFile(atPath: executableURL.path) else {
            let failure = LocalServerSupervisorError.bundledServerNotExecutable(executableURL)
            retainedLaunchFailure = failure
            throw failure
        }

        let capture = BoundedStandardErrorCapture(maxBytes: maxStandardErrorBytes)
        let controlPipe = Pipe()
        let launch: OwnedLaunch

        do {
            let process = try ExactChildProcess.spawn(
                executableURL: executableURL,
                arguments: ["serve", "--app-control-stdin"],
                standardInput: controlPipe.fileHandleForReading,
                standardError: capture.writer
            )
            launch = OwnedLaunch(
                process: process,
                standardErrorCapture: capture,
                controlWriter: controlPipe.fileHandleForWriting
            )
            try? controlPipe.fileHandleForReading.close()
            capture.closeParentWriter()
            ownedLaunch = launch
        } catch {
            try? controlPipe.fileHandleForReading.close()
            try? controlPipe.fileHandleForWriting.close()
            capture.closeParentWriter()
            let message = Self.sanitizedLaunchError(error.localizedDescription)
            let failure = LocalServerSupervisorError.launchFailed(message)
            retainedLaunchFailure = failure
            throw failure
        }

        if startupGraceNanoseconds > 0 {
            try? await Task.sleep(nanoseconds: startupGraceNanoseconds)
        } else {
            await Task.yield()
        }

        guard !launch.process.isRunning else { return }
        let failure = await failureForExitedProcess(launch.process, capture: capture)
        if ownedLaunch === launch {
            launch.closeControlChannel()
            ownedLaunch = nil
            if !launch.stopRequested {
                retainedLaunchFailure = failure
            }
        }
        if !launch.stopRequested { throw failure }
    }

    /// Clears a crash-loop latch only when the app has an explicit user or
    /// profile-activation intent to try Local again. Automatic reconnects never
    /// call this and therefore cannot repeatedly respawn a broken helper.
    func prepareForExplicitRetry() {
        retainedLaunchFailure = nil
    }

    func stopOwnedServer() {
        guard let launch = ownedLaunch,
              launch.process.isRunning,
              !launch.stopRequested else { return }
        launch.stopRequested = true
        launch.interruptWasSent = true

        // The server handles INT as a graceful shutdown and removes the Unix
        // socket on exit. A bounded task escalates only against the exact,
        // exclusively reaped child if its control reader or shutdown hangs.
        launch.send(.interrupt)
        normalStopTask = Task { @MainActor [weak self] in
            guard let self else { return }
            await stopExactly(launch)
            if ownedLaunch === launch {
                launch.closeControlChannel()
                ownedLaunch = nil
            }
            normalStopTask = nil
        }
    }

    /// App termination is the only path that escalates beyond the server's
    /// normal graceful SIGINT. Cocoa waits for this method before terminating,
    /// so an exact child that ignores graceful shutdown cannot be orphaned.
    func shutdownOwnedServerForAppTermination() async {
        applicationShutdownRequested = true

        if let applicationShutdownTask {
            await applicationShutdownTask.value
            return
        }

        if let normalStopTask {
            applicationShutdownTask = normalStopTask
            await normalStopTask.value
            return
        }

        guard let launch = ownedLaunch, launch.process.isRunning else {
            ownedLaunch?.closeControlChannel()
            ownedLaunch = nil
            return
        }

        launch.stopRequested = true
        let shutdownTask = Task { @MainActor [weak self] in
            guard let self else { return }
            await stopExactly(launch)
            if ownedLaunch === launch {
                launch.closeControlChannel()
                ownedLaunch = nil
            }
        }
        applicationShutdownTask = shutdownTask
        await shutdownTask.value
    }

    private func stopExactly(_ launch: OwnedLaunch) async {
        defer { launch.closeControlChannel() }

        if !launch.interruptWasSent {
            launch.interruptWasSent = true
            send(.interrupt, to: launch)
        }
        let exitedAfterInterrupt = await waitForExit(
            of: launch,
            timeoutNanoseconds: interruptShutdownGraceNanoseconds
        )
        guard !exitedAfterInterrupt else { return }

        send(.terminate, to: launch)
        let exitedAfterTerminate = await waitForExit(
            of: launch,
            timeoutNanoseconds: terminateShutdownGraceNanoseconds
        )
        guard !exitedAfterTerminate else { return }

        send(.kill, to: launch)
        let exitedAfterControlKill = await waitForExit(
            of: launch,
            timeoutNanoseconds: controlKillShutdownGraceNanoseconds
        )
        guard !exitedAfterControlKill else { return }

        // The pipe reader may itself be stopped or broken. ExactChildProcess is
        // the sole reaper, so this liveness-check-plus-SIGKILL has no PID reuse
        // window even if the child exits concurrently and becomes a zombie.
        let process = launch.process
        if !process.sendSignal(SIGKILL) {
            logSupervisorFailure("failed to SIGKILL owned local helper")
        }
        await Task.detached(priority: .userInitiated) {
            process.waitUntilExit()
        }.value
    }

    private func send(_ command: AppControlCommand, to launch: OwnedLaunch) {
        guard ownedLaunch === launch else { return }
        launch.send(command)
    }

    private func waitForExit(
        of launch: OwnedLaunch,
        timeoutNanoseconds: UInt64
    ) async -> Bool {
        let started = DispatchTime.now().uptimeNanoseconds
        while ownedLaunch === launch, launch.process.isRunning {
            let elapsed = DispatchTime.now().uptimeNanoseconds - started
            guard elapsed < timeoutNanoseconds else { return false }
            try? await Task.sleep(
                nanoseconds: min(shutdownPollIntervalNanoseconds, timeoutNanoseconds - elapsed)
            )
        }
        return true
    }

    private func failureForExitedProcess(
        _ process: ExactChildProcess,
        capture: BoundedStandardErrorCapture
    ) async -> LocalServerSupervisorError {
        let diagnostic = await capture.diagnostic()
        return .bundledServerExited(
            reason: process.terminationReason(),
            diagnostic: diagnostic
        )
    }

    private func logSupervisorFailure(_ message: String) {
        NSLog("Cowchat local server supervisor: %@", message)
    }

    private nonisolated static func sanitizedLaunchError(_ message: String) -> String {
        message
            .split(whereSeparator: \Character.isNewline)
            .joined(separator: " ")
            .prefix(512)
            .description
    }

    nonisolated static func serverExecutableURL(in bundleURL: URL) -> URL {
        bundleURL
            .appendingPathComponent("Contents", isDirectory: true)
            .appendingPathComponent("Helpers", isDirectory: true)
            .appendingPathComponent("cowchat-server", isDirectory: false)
    }
}
