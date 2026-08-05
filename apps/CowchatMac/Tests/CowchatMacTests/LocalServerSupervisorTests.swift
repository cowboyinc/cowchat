import Darwin
import XCTest
@testable import CowchatMac

final class LocalServerSupervisorTests: XCTestCase {
    private var temporaryDirectories: [URL] = []

    override func tearDownWithError() throws {
        for directory in temporaryDirectories {
            try? FileManager.default.removeItem(at: directory)
        }
        temporaryDirectories = []
    }

    func testBundledServerUsesFixedHelpersPath() {
        let bundle = URL(fileURLWithPath: "/Applications/Cowchat.app", isDirectory: true)

        XCTAssertEqual(
            LocalServerSupervisor.serverExecutableURL(in: bundle).path,
            "/Applications/Cowchat.app/Contents/Helpers/cowchat-server"
        )
    }

    @MainActor
    func testMissingBundledServerFailsWithoutClaimingOwnership() async {
        let missingBundle = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let supervisor = LocalServerSupervisor(bundleURL: missingBundle)

        do {
            try await supervisor.launchIfNeeded()
            XCTFail("expected a missing helper error")
        } catch LocalServerSupervisorError.missingBundledServer {
            // Expected.
        } catch {
            XCTFail("unexpected error: \(error)")
        }
        XCTAssertFalse(supervisor.ownsRunningServer)
    }

    @MainActor
    func testRunningBundledServerLaunchesOnlyOnce() async throws {
        let invocationFile = temporaryFileURL(named: "invocations")
        let fixture = try makeBundleFixture(script: """
        echo launched >> '\(invocationFile.path)'
        trap 'exit 0' INT TERM
        start_app_control
        while :; do sleep 0.05; done
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 250_000_000
        )
        defer { supervisor.stopOwnedServer() }

        try await supervisor.launchIfNeeded()
        try await supervisor.launchIfNeeded()
        await waitUntil { (try? self.lineCount(in: invocationFile)) == 1 }
        XCTAssertTrue(supervisor.ownsRunningServer)
        XCTAssertEqual(try String(contentsOf: invocationFile, encoding: .utf8).split(separator: "\n").count, 1)

        supervisor.stopOwnedServer()
        await waitUntil { !supervisor.ownsRunningServer }
    }

    @MainActor
    func testEarlyExitReturnsDiagnosticAndDoesNotRespawn() async throws {
        let invocationFile = temporaryFileURL(named: "early-exit-invocations")
        let fixture = try makeBundleFixture(script: """
        echo launched >> '\(invocationFile.path)'
        echo 'bind failed: address is already in use' >&2
        exit 73
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 250_000_000
        )

        let firstError = await launchError(from: supervisor)
        XCTAssertTrue(firstError.contains("status 73"), firstError)
        XCTAssertTrue(firstError.contains("address is already in use"), firstError)

        let secondError = await launchError(from: supervisor)
        XCTAssertEqual(secondError, firstError)
        XCTAssertEqual(try String(contentsOf: invocationFile, encoding: .utf8).split(separator: "\n").count, 1)
        XCTAssertFalse(supervisor.ownsRunningServer)
    }

    @MainActor
    func testExplicitRetryLaunchesAgainAfterTransientEarlyExit() async throws {
        let firstAttemptFile = temporaryFileURL(named: "first-attempt")
        let invocationFile = temporaryFileURL(named: "explicit-retry-invocations")
        let fixture = try makeBundleFixture(script: """
        echo launched >> '\(invocationFile.path)'
        if [ ! -e '\(firstAttemptFile.path)' ]; then
          touch '\(firstAttemptFile.path)'
          echo 'transient startup failure' >&2
          exit 75
        fi
        trap 'exit 0' INT TERM
        start_app_control
        while :; do sleep 0.05; done
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 250_000_000
        )
        defer { supervisor.stopOwnedServer() }

        let firstError = await launchError(from: supervisor)
        let automaticRetryError = await launchError(from: supervisor)
        XCTAssertEqual(automaticRetryError, firstError)
        XCTAssertEqual(try lineCount(in: invocationFile), 1)

        supervisor.prepareForExplicitRetry()
        try await supervisor.launchIfNeeded()
        await waitUntil { (try? self.lineCount(in: invocationFile)) == 2 }

        XCTAssertTrue(supervisor.ownsRunningServer)
        XCTAssertEqual(try lineCount(in: invocationFile), 2)

        supervisor.stopOwnedServer()
        await waitUntil { !supervisor.ownsRunningServer }
    }

    @MainActor
    func testEarlyExitDiagnosticIsBoundedAndRedactsLikelySecrets() async throws {
        let fixture = try makeBundleFixture(script: """
        printf 'api_key=super-secret-value ' 1>&2
        printf 'Bearer also-secret ' 1>&2
        printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' 1>&2
        exit 2
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 250_000_000,
            maxStandardErrorBytes: 80
        )

        let error = await launchError(from: supervisor)
        XCTAssertFalse(error.contains("super-secret-value"), error)
        XCTAssertFalse(error.contains("also-secret"), error)
        XCTAssertTrue(error.contains("api_key=[REDACTED]"), error)
        XCTAssertTrue(error.contains("Bearer [REDACTED]"), error)
        XCTAssertTrue(error.contains("…"), error)
        XCTAssertLessThan(error.utf8.count, 500)
    }

    @MainActor
    func testStopSendsSIGINTOnlyToOwnedHelper() async throws {
        let signalFile = temporaryFileURL(named: "signal")
        let commandFile = temporaryFileURL(named: "control-commands")
        let readyFile = temporaryFileURL(named: "ready-for-signal")
        let fixture = try makeBundleFixture(script: """
        trap "echo SIGINT > '\(signalFile.path)'; exit 0" INT
        start_app_control '\(commandFile.path)'
        touch '\(readyFile.path)'
        while :; do sleep 0.05; done
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 250_000_000
        )
        defer { supervisor.stopOwnedServer() }

        try await supervisor.launchIfNeeded()
        XCTAssertTrue(supervisor.ownsRunningServer)
        await waitUntil { FileManager.default.fileExists(atPath: readyFile.path) }
        supervisor.stopOwnedServer()

        await waitUntil { FileManager.default.fileExists(atPath: signalFile.path) }
        XCTAssertEqual(
            try String(contentsOf: signalFile, encoding: .utf8)
                .trimmingCharacters(in: .whitespacesAndNewlines),
            "SIGINT"
        )
        await waitUntil { !supervisor.ownsRunningServer }
        XCTAssertEqual(try controlCommands(in: commandFile), ["INT"])
    }

    @MainActor
    func testAppTerminationKillsOwnedHelperThatIgnoresGracefulSignals() async throws {
        let invocationFile = temporaryFileURL(named: "shutdown-invocations")
        let commandFile = temporaryFileURL(named: "shutdown-control-commands")
        let processIDFile = temporaryFileURL(named: "shutdown-pid")
        let readyFile = temporaryFileURL(named: "ready-for-shutdown")
        let fixture = try makeBundleFixture(script: """
        echo launched >> '\(invocationFile.path)'
        trap '' INT TERM
        start_app_control '\(commandFile.path)'
        echo $$ > '\(processIDFile.path)'
        touch '\(readyFile.path)'
        while :; do sleep 0.05; done
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 100_000_000,
            interruptShutdownGraceNanoseconds: 30_000_000,
            terminateShutdownGraceNanoseconds: 30_000_000,
            shutdownPollIntervalNanoseconds: 2_000_000
        )

        try await supervisor.launchIfNeeded()
        await waitUntil { FileManager.default.fileExists(atPath: readyFile.path) }
        let processIDText = try String(contentsOf: processIDFile, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let processID = try XCTUnwrap(Int32(processIDText))

        await supervisor.shutdownOwnedServerForAppTermination()

        XCTAssertFalse(supervisor.ownsRunningServer)
        errno = 0
        XCTAssertEqual(Darwin.kill(processID, 0), -1)
        XCTAssertEqual(errno, ESRCH)
        XCTAssertEqual(try lineCount(in: invocationFile), 1)
        XCTAssertEqual(try controlCommands(in: commandFile), ["INT", "TERM", "KILL"])

        supervisor.prepareForExplicitRetry()
        do {
            try await supervisor.launchIfNeeded()
            XCTFail("app termination must permanently fence helper relaunch")
        } catch LocalServerSupervisorError.bundledServerStopping {
            // Expected: shutdown cannot race with an automatic or explicit relaunch.
        } catch {
            XCTFail("unexpected error: \(error)")
        }
        XCTAssertEqual(try lineCount(in: invocationFile), 1)
    }

    @MainActor
    func testAppTerminationForceKillsExactHelperWithUnreadControlPipe() async throws {
        let processIDFile = temporaryFileURL(named: "unread-shutdown-pid")
        let readyFile = temporaryFileURL(named: "unread-ready-for-shutdown")
        let fixture = try makeBundleFixture(script: """
        trap '' INT TERM
        echo $$ > '\(processIDFile.path)'
        touch '\(readyFile.path)'
        while :; do sleep 0.05; done
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 100_000_000,
            interruptShutdownGraceNanoseconds: 30_000_000,
            terminateShutdownGraceNanoseconds: 30_000_000,
            controlKillShutdownGraceNanoseconds: 30_000_000,
            shutdownPollIntervalNanoseconds: 2_000_000
        )

        try await supervisor.launchIfNeeded()
        await waitUntil { FileManager.default.fileExists(atPath: readyFile.path) }
        let processIDText = try String(contentsOf: processIDFile, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let processID = try XCTUnwrap(Int32(processIDText))

        await supervisor.shutdownOwnedServerForAppTermination()

        XCTAssertFalse(supervisor.ownsRunningServer)
        errno = 0
        XCTAssertEqual(Darwin.kill(processID, 0), -1)
        XCTAssertEqual(errno, ESRCH)
    }

    @MainActor
    func testOrdinaryStopForceKillsUnreadPipeThenRelaunchesLocal() async throws {
        let invocationFile = temporaryFileURL(named: "unread-stop-invocations")
        let processIDFile = temporaryFileURL(named: "unread-stop-pids")
        let fixture = try makeBundleFixture(script: """
        echo launched >> '\(invocationFile.path)'
        echo $$ >> '\(processIDFile.path)'
        trap '' INT TERM
        while :; do sleep 0.05; done
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 100_000_000,
            interruptShutdownGraceNanoseconds: 30_000_000,
            terminateShutdownGraceNanoseconds: 30_000_000,
            controlKillShutdownGraceNanoseconds: 30_000_000,
            shutdownPollIntervalNanoseconds: 2_000_000
        )

        try await supervisor.launchIfNeeded()
        await waitUntil { (try? self.lineCount(in: invocationFile)) == 1 }
        let firstProcessID = try XCTUnwrap(processIDs(in: processIDFile).first)

        supervisor.stopOwnedServer()
        supervisor.prepareForExplicitRetry()
        try await supervisor.launchIfNeeded()

        await waitUntil { (try? self.lineCount(in: invocationFile)) == 2 }
        XCTAssertTrue(supervisor.ownsRunningServer)
        XCTAssertEqual(try lineCount(in: invocationFile), 2)
        errno = 0
        XCTAssertEqual(Darwin.kill(firstProcessID, 0), -1)
        XCTAssertEqual(errno, ESRCH)

        let secondProcessID = try XCTUnwrap(processIDs(in: processIDFile).last)
        await supervisor.shutdownOwnedServerForAppTermination()
        errno = 0
        XCTAssertEqual(Darwin.kill(secondProcessID, 0), -1)
        XCTAssertEqual(errno, ESRCH)
    }

    @MainActor
    func testAppTerminationFencesRelaunchWaitingForNormalStop() async throws {
        let invocationFile = temporaryFileURL(named: "termination-race-invocations")
        let stoppingFile = temporaryFileURL(named: "termination-race-stopping")
        let fixture = try makeBundleFixture(script: """
        echo launched >> '\(invocationFile.path)'
        trap "touch '\(stoppingFile.path)'; sleep 0.35; exit 0" INT
        start_app_control
        while :; do sleep 0.05; done
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 100_000_000,
            shutdownPollIntervalNanoseconds: 2_000_000
        )

        try await supervisor.launchIfNeeded()
        await waitUntil { (try? self.lineCount(in: invocationFile)) == 1 }
        supervisor.stopOwnedServer()
        await waitUntil { FileManager.default.fileExists(atPath: stoppingFile.path) }
        supervisor.prepareForExplicitRetry()

        let relaunchEntered = expectation(description: "relaunch entered")
        let relaunchTask = Task { @MainActor in
            relaunchEntered.fulfill()
            try await supervisor.launchIfNeeded()
        }
        await fulfillment(of: [relaunchEntered])

        await supervisor.shutdownOwnedServerForAppTermination()

        do {
            try await relaunchTask.value
            XCTFail("app termination must fence a relaunch suspended on normal stop")
        } catch LocalServerSupervisorError.bundledServerStopping {
            // Expected after the post-await shutdown-fence recheck.
        } catch {
            XCTFail("unexpected error: \(error)")
        }
        XCTAssertFalse(supervisor.ownsRunningServer)
        XCTAssertEqual(try lineCount(in: invocationFile), 1)
    }

    @MainActor
    func testStopInProgressWaitsForExactChildThenRelaunchesOnce() async throws {
        let invocationFile = temporaryFileURL(named: "stop-relaunch-invocations")
        let stoppingFile = temporaryFileURL(named: "stopping")
        let fixture = try makeBundleFixture(script: """
        echo launched >> '\(invocationFile.path)'
        trap "touch '\(stoppingFile.path)'; sleep 0.35; exit 0" INT
        start_app_control
        while :; do sleep 0.05; done
        """)
        let supervisor = LocalServerSupervisor(
            bundleURL: fixture,
            startupGraceNanoseconds: 250_000_000
        )
        defer { supervisor.stopOwnedServer() }

        try await supervisor.launchIfNeeded()
        await waitUntil { (try? self.lineCount(in: invocationFile)) == 1 }
        XCTAssertEqual(try lineCount(in: invocationFile), 1)
        supervisor.stopOwnedServer()
        await waitUntil { FileManager.default.fileExists(atPath: stoppingFile.path) }
        supervisor.prepareForExplicitRetry()

        // Returning to Local waits for the exact stopping child instead of
        // surfacing a permanent stopping error or starting a concurrent copy.
        try await supervisor.launchIfNeeded()
        await waitUntil { (try? self.lineCount(in: invocationFile)) == 2 }

        XCTAssertTrue(supervisor.ownsRunningServer)
        XCTAssertEqual(try lineCount(in: invocationFile), 2)

        supervisor.stopOwnedServer()
        await waitUntil { !supervisor.ownsRunningServer }
    }

    @MainActor
    private func launchError(from supervisor: LocalServerSupervisor) async -> String {
        do {
            try await supervisor.launchIfNeeded()
            XCTFail("expected helper launch to fail")
            return ""
        } catch {
            return error.localizedDescription
        }
    }

    private func makeBundleFixture(script: String) throws -> URL {
        let bundle = FileManager.default.temporaryDirectory
            .appendingPathComponent("CowchatSupervisorTests-\(UUID().uuidString)", isDirectory: true)
            .appendingPathExtension("app")
        temporaryDirectories.append(bundle)
        let helper = LocalServerSupervisor.serverExecutableURL(in: bundle)
        try FileManager.default.createDirectory(
            at: helper.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let contents = """
        #!/bin/sh
        if [ "$#" -ne 2 ] || [ "$1" != "serve" ] || [ "$2" != "--app-control-stdin" ]; then
          echo "unexpected helper arguments" >&2
          exit 64
        fi

        start_app_control() {
          (
            app_control_log=$1
            while IFS= read -r app_control_command; do
              if [ -n "$app_control_log" ]; then
                printf '%s\\n' "$app_control_command" >> "$app_control_log"
              fi
              case "$app_control_command" in
                INT) kill -INT "$$" ;;
                TERM) kill -TERM "$$" ;;
                KILL) kill -KILL "$$"; exit 0 ;;
              esac
            done
          ) <&0 >/dev/null 2>&1 &
        }

        \(script)
        """
        try contents.write(to: helper, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: helper.path)
        return bundle
    }

    private func temporaryFileURL(named name: String) -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("CowchatSupervisorTests-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        temporaryDirectories.append(directory)
        return directory.appendingPathComponent(name)
    }

    private func lineCount(in file: URL) throws -> Int {
        try String(contentsOf: file, encoding: .utf8).split(separator: "\n").count
    }

    private func controlCommands(in file: URL) throws -> [String] {
        try String(contentsOf: file, encoding: .utf8)
            .split(separator: "\n")
            .map(String.init)
    }

    private func processIDs(in file: URL) throws -> [Int32] {
        try String(contentsOf: file, encoding: .utf8)
            .split(separator: "\n")
            .compactMap { Int32($0) }
    }

    @MainActor
    private func waitUntil(
        timeoutNanoseconds: UInt64 = 2_000_000_000,
        condition: @MainActor @escaping () -> Bool
    ) async {
        let started = DispatchTime.now().uptimeNanoseconds
        while !condition(), DispatchTime.now().uptimeNanoseconds - started < timeoutNanoseconds {
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertTrue(condition())
    }
}
