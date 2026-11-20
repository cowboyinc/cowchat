import XCTest

final class LiveServerTests: XCTestCase {
    private let app = XCUIApplication()

    override func setUp() {
        continueAfterFailure = false
    }

    /// The account screens over the debug build's made-up account. No server and no sign-in.
    func testAccountScreens() {
        app.launchArguments = ["-accountScreens"]
        app.launch()

        app.buttons["harness.inbox"].tap()
        XCTAssertTrue(app.staticTexts["Invited you to join."].waitForExistence(timeout: 10), "an open invitation shows; on screen: \(visibleText())")
        XCTAssertTrue(app.staticTexts["could you review the plan"].exists, "a mention shows without the leading @name")
        XCTAssertTrue(app.staticTexts["Cal's cowchat-main"].exists, "a session's marked message names its owner")
        app.buttons["inbox.markAllRead"].firstMatch.tap()
        XCTAssertTrue(waitFor(timeout: 5) { !self.app.buttons["inbox.markAllRead"].exists }, "Mark all read leaves nothing to mark")
        app.staticTexts["Invited you to join."].tap()
        XCTAssertTrue(app.buttons["Decline"].waitForExistence(timeout: 5), "an invitation asks to join or decline")
        app.buttons["Decline"].tap()
        XCTAssertTrue(waitFor(timeout: 5) { !self.app.staticTexts["Invited you to join."].exists }, "a declined invitation leaves the Inbox")
        app.buttons["Done"].tap()

        app.buttons["harness.friends"].tap()
        XCTAssertTrue(app.buttons["Accept"].waitForExistence(timeout: 5), "a friend request shows; on screen: \(visibleText())")
        app.buttons["Accept"].tap()
        XCTAssertTrue(app.staticTexts["1 friend"].waitForExistence(timeout: 5), "accepting adds the friend")
        type("nobody", into: app.textFields["friends.handle"])
        app.buttons["friends.add"].tap()
        XCTAssertTrue(app.alerts.firstMatch.waitForExistence(timeout: 5), "a refused friend request says so; on screen: \(visibleText())")
        XCTAssertTrue(app.alerts.staticTexts["not found"].exists)
        app.alerts.buttons["OK"].tap()
        app.textFields["friends.handle"].tap()
        app.textFields["friends.handle"].typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 6) + "sam")
        app.buttons["friends.add"].tap()
        XCTAssertTrue(app.staticTexts["Sent"].waitForExistence(timeout: 5), "a sent request is listed")
        app.buttons["Done"].tap()

        app.buttons["harness.team"].tap()
        XCTAssertTrue(app.staticTexts["feature/inbox"].waitForExistence(timeout: 5), "a teammate's branch shows; on screen: \(visibleText())")
        XCTAssertTrue(app.staticTexts["Waiting"].exists)
        XCTAssertTrue(app.staticTexts["cowchat #7 · Checks running"].exists)
        app.buttons["Done"].tap()

        app.buttons["harness.profile"].tap()
        let name = app.textFields["profile.name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        XCTAssertEqual(name.value as? String, "Ryan Chan")
        XCTAssertFalse(app.buttons["profile.save"].isEnabled, "Save waits for a change")
        type(" Jr", into: name)
        app.buttons["profile.save"].tap()
        XCTAssertTrue(waitFor(timeout: 5) { !self.app.buttons["profile.save"].isEnabled }, "saving settles the form")
        app.buttons["Done"].tap()

        app.buttons["harness.setup"].tap()
        let handle = app.textFields["setup.handle"]
        XCTAssertTrue(handle.waitForExistence(timeout: 5), "setup asks for a name and username; on screen: \(visibleText())")
        handle.tap()
        handle.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 12) + "cal")
        XCTAssertTrue(app.staticTexts["That username is taken. Try another."].waitForExistence(timeout: 5), "a taken username is refused")
        XCTAssertFalse(app.buttons["setup.continue"].isEnabled)
        handle.typeText("wick")
        XCTAssertTrue(app.staticTexts["Username available."].waitForExistence(timeout: 5))
        app.buttons["setup.continue"].tap()
        XCTAssertTrue(waitFor(timeout: 5) { !self.app.buttons["setup.continue"].exists }, "setup closes once it is done")

        app.buttons["harness.people"].tap()
        XCTAssertTrue(app.staticTexts["chad_bot"].waitForExistence(timeout: 5), "an account room lists its agents; on screen: \(visibleText())")
        XCTAssertTrue(app.staticTexts["@cal"].exists, "and its people")
        XCTAssertEqual(app.buttons.matching(identifier: "people.removeAgent").count, 1, "only your own agent can be taken out of someone else's room")
        app.buttons["people.removeAgent"].firstMatch.tap()
        XCTAssertTrue(app.buttons["people.addAgent"].waitForExistence(timeout: 5), "an agent taken out can be added back")
        app.buttons["Done"].tap()

        app.buttons["harness.friendLink"].tap()
        XCTAssertTrue(app.buttons["friendLink.send"].waitForExistence(timeout: 5), "a friend link shows who it is; on screen: \(visibleText())")
        XCTAssertTrue(app.staticTexts["Cal Fisher"].exists)
        app.buttons["friendLink.send"].tap()
        XCTAssertTrue(app.staticTexts["Request sent."].waitForExistence(timeout: 5))
        app.buttons["Done"].tap()
    }

    /// The real app over the debug build's made-up server and account: the tabs a signed-in account gets, the agent
    /// mark, folded agent messages. No server and no sign-in.
    func testASignedInAccountsTabs() {
        app.launchArguments = ["-demoAccount"]
        app.launch()
        XCTAssertTrue(waitFor(timeout: 15) { self.tab("Inbox").exists && self.tab("People").exists && self.tab("You").exists },
                      "an account has Inbox, People and You; buttons: \(app.buttons.allElementsBoundByIndex.map(\.label).prefix(12))")

        let design = app.staticTexts["design"]
        XCTAssertTrue(design.waitForExistence(timeout: 10), "the account's rooms are listed")
        press(design)
        XCTAssertTrue(app.staticTexts["Tests pass. Ready for review."].waitForExistence(timeout: 10), "an agent message marked for people shows")
        XCTAssertTrue(app.buttons["room.agentRun"].firstMatch.exists, "the agent's other messages are folded")
        XCTAssertFalse(app.staticTexts["running the linter"].exists)
        XCTAssertEqual(app.descendants(matching: .any).matching(identifier: "message.agentMark").count, 1,
                       "the agent's message carries the mark, the person's messages do not")
        app.buttons["room.agentRun"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["running the linter"].waitForExistence(timeout: 5), "the folded run opens in place")

        openTab("Inbox")
        XCTAssertTrue(app.staticTexts["Invited you to join."].waitForExistence(timeout: 10), "the Inbox tab lists an invitation")
        XCTAssertTrue(app.staticTexts["could you review the plan"].exists)

        openTab("People")
        XCTAssertTrue(app.staticTexts["Friends"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["Accept"].exists, "a friend request shows under Friends")
        XCTAssertTrue(app.staticTexts["Team"].exists)
        XCTAssertTrue(app.staticTexts["feature/inbox"].exists, "a teammate's branch shows under Team")

        openTab("You")
        XCTAssertTrue(app.textFields["profile.name"].waitForExistence(timeout: 10), "You shows the profile")
        XCTAssertEqual(app.textFields["profile.name"].value as? String, "Ryan Chan")
        XCTAssertTrue(app.switches["notifications.toggle"].exists)
        XCTAssertTrue(text(containing: "@ryan").waitForExistence(timeout: 5), "the connection is named by the account's handle")

        openTab("Inbox")
        app.staticTexts["could you review the plan"].tap()
        XCTAssertTrue(app.buttons["composer.send"].waitForExistence(timeout: 10), "an Inbox item opens its room on the Rooms tab")
    }

    /// For a real phone that is already signed in to an account. It reads screens and asks the server for the
    /// person's own friend link, then sends a friend request to a made-up handle that the server refuses. It removes
    /// nothing. Skipped unless asked for.
    func testSignedInAccountOnARealPhone() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["COWCHAT_REAL_ACCOUNT"] == "1", "set TEST_RUNNER_COWCHAT_REAL_ACCOUNT=1")
        app.launch()
        XCTAssertTrue(waitFor(timeout: 20) { self.tab("Inbox").exists }, "a signed-in account has an Inbox tab; buttons: \(app.buttons.allElementsBoundByIndex.map(\.label).prefix(12))")
        openTab("Inbox")
        XCTAssertTrue(app.navigationBars["Inbox"].waitForExistence(timeout: 10), "the Inbox opens")
        sleep(4)
        print("REAL_INBOX_LOAD_FAILED: \(app.staticTexts["account.loadFailed"].exists)")
        print("REAL_INBOX_ROWS: \(app.buttons.matching(identifier: "inbox.item").count)")

        openTab("People")
        XCTAssertTrue(app.buttons["friends.shareLink"].waitForExistence(timeout: 10))
        sleep(3)
        print("REAL_FRIENDS_LOAD_FAILED: \(app.staticTexts["account.loadFailed"].exists)")
        app.buttons["friends.shareLink"].tap()
        let closeShare = app.navigationBars["UIActivityContentView"].buttons["Close"]
        let shared = waitFor(timeout: 15) { closeShare.exists || self.app.otherElements["ActivityListView"].exists }
        print("REAL_FRIEND_LINK_SHARE_SHEET: \(shared)")
        let alert = app.alerts.firstMatch
        if alert.exists {
            print("REAL_FRIEND_LINK_ERROR: \(alert.staticTexts.allElementsBoundByIndex.map(\.label))")
            alert.buttons["OK"].tap()
        }
        if closeShare.exists { closeShare.tap() } else if shared { app.swipeDown(velocity: .fast) }
        XCTAssertTrue(shared, "Share my friend link opens the share sheet with the link from the server")

        // A request to a handle nobody has: the server should answer and refuse, and nobody receives anything.
        let nobody = "zz-ios-check-" + UUID().uuidString.prefix(18).lowercased()
        type(nobody, into: app.textFields["friends.handle"])
        app.buttons["friends.add"].tap()
        let refusal = app.alerts.firstMatch
        let refused = refusal.waitForExistence(timeout: 15)
        print("REAL_FRIEND_REQUEST_TO_NOBODY_REFUSED: \(refused) \(refused ? refusal.staticTexts.allElementsBoundByIndex.map(\.label) : [])")
        print("REAL_FRIEND_REQUEST_SENT_ROW: \(app.staticTexts["Sent"].exists)")
        if refused { refusal.buttons["OK"].tap() }
        XCTAssertTrue(refused, "the server refuses a request to a handle nobody has")
        // Friends and Team share the People tab, so one look covers both.
        print("REAL_TEAM_LOAD_FAILED: \(app.staticTexts["account.loadFailed"].exists)")

        openConnections()
        XCTAssertTrue(app.buttons["connections.row"].firstMatch.waitForExistence(timeout: 10))
        // Only how each connection was made, not its name or address.
        let kinds = app.staticTexts.allElementsBoundByIndex.map(\.label).compactMap { label in
            ["@", "Account", "Invited", "Key"].first { label.hasPrefix($0) }
        }
        print("REAL_CONNECTION_KINDS: \(kinds)")
        XCTAssertTrue(kinds.contains("@"), "the account connection shows its handle")
    }

    /// On an iPad the room list stays beside the open room. Needs the seeded test server; skipped unless asked for.
    func testPadShowsTheListBesideTheOpenRoom() throws {
        let env = ProcessInfo.processInfo.environment
        try XCTSkipUnless(env["COWCHAT_PAD"] == "1", "set TEST_RUNNER_COWCHAT_PAD=1 and run on an iPad")
        let key = try XCTUnwrap(env["COWCHAT_TEST_KEY"])
        let port = env["COWCHAT_TEST_PORT"] ?? "9339"
        app.launch()
        signOutIfNeeded()
        fillOwnServer(port: port)
        type(key, into: app.secureTextFields["signin.paste"])
        app.buttons["signin.connect"].tap()

        let review = app.staticTexts["design-review"]
        XCTAssertTrue(review.waitForExistence(timeout: 15), "room list after sign-in")
        for label in ["Not Now", "Not now"] where app.buttons[label].waitForExistence(timeout: 2) { app.buttons[label].tap() }
        press(review)
        let send = app.buttons["composer.send"]
        if !send.waitForExistence(timeout: 5) { press(review) }
        XCTAssertTrue(send.waitForExistence(timeout: 10), "the room opens")
        XCTAssertTrue(app.staticTexts["sealed"].isHittable, "the list is still on screen beside the open room")
        XCTAssertTrue(app.staticTexts["room.presence"].exists)

        let composer = app.textFields["composer.field"].exists ? app.textFields["composer.field"] : app.textViews["composer.field"]
        composer.tap()
        composer.typeText("hello from the pad")
        send.tap()
        XCTAssertTrue(text(containing: "hello from the pad").waitForExistence(timeout: 10))

        press(app.staticTexts["sealed"])
        XCTAssertTrue(app.buttons["Enter room secret"].firstMatch.waitForExistence(timeout: 10), "another room replaces it")
        XCTAssertTrue(review.isHittable, "and the list is still there")

        press(review)
        XCTAssertTrue(send.waitForExistence(timeout: 10))
        XCUIDevice.shared.orientation = .landscapeLeft
        sleep(2)
        XCTAssertTrue(send.isHittable, "the open room survives turning the iPad")
        XCTAssertTrue(app.staticTexts["sealed"].isHittable, "and so does the list")
        XCTAssertTrue(text(containing: "hello from the pad").waitForExistence(timeout: 8), "the newest message is still in view")
    }

    func testSignInReadSendUnreadSecretAndReconnect() throws {
        let env = ProcessInfo.processInfo.environment
        let key = try XCTUnwrap(env["COWCHAT_TEST_KEY"], "set TEST_RUNNER_COWCHAT_TEST_KEY")
        let port = env["COWCHAT_TEST_PORT"] ?? "9339"
        let nonce = env["COWCHAT_TEST_NONCE"] ?? "local"

        app.launch()
        signOutIfNeeded()

        fillOwnServer(port: port)
        type(key, into: app.secureTextFields["signin.paste"])
        XCTAssertTrue(app.staticTexts["Key"].waitForExistence(timeout: 5), "the paste is recognised as a key")
        app.buttons["signin.connect"].tap()

        let reviewRow = app.staticTexts["design-review"]
        XCTAssertTrue(reviewRow.waitForExistence(timeout: 15), "room list after sign-in; on screen: \(visibleText())")
        XCTAssertTrue(app.staticTexts["sealed"].exists)

        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        for label in ["Not Now", "Not now", "Don’t Allow"] where springboard.buttons[label].waitForExistence(timeout: 2) {
            print("SYSTEM_PROMPT_DISMISSED: \(springboard.staticTexts.allElementsBoundByIndex.map(\.label).prefix(4))")
            springboard.buttons[label].tap()
        }
        for label in ["Not Now", "Not now"] where app.buttons[label].exists {
            print("APP_PROMPT_DISMISSED: \(label)")
            app.buttons[label].tap()
            sleep(2)
        }
        press(reviewRow)
        if !app.buttons["composer.send"].waitForExistence(timeout: 5) { press(reviewRow) }
        XCTAssertTrue(text(containing: "live ping").waitForExistence(timeout: 20), "history and live messages; on screen: \(visibleText())")
        XCTAssertTrue(text(containing: "Review of PR 12").waitForExistence(timeout: 30), "markdown renders without asterisks; on screen: \(visibleText())")
        XCTAssertTrue(text(containing: "let x = timeline.merge(page)").exists, "code block")

        let composer = app.textFields["composer.field"].exists ? app.textFields["composer.field"] : app.textViews["composer.field"]
        composer.tap()
        composer.typeText("hello from phone \(nonce) @ada")
        let suggestion = app.buttons["@ada-agent"]
        XCTAssertTrue(suggestion.waitForExistence(timeout: 5), "mention suggestion")
        suggestion.tap()
        app.buttons["composer.send"].tap()
        XCTAssertTrue(text(containing: "hello from phone \(nonce)").waitForExistence(timeout: 10), "sent message shows once")
        XCTAssertEqual(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "hello from phone \(nonce)")).count, 1)

        let presence = app.staticTexts["room.presence"]
        XCTAssertTrue(
            waitFor(timeout: 10) { presence.exists && (presence.label.contains("connected") || presence.label.contains("active")) },
            "the header shows who is connected; header: \(presence.exists ? presence.label : "missing")"
        )
        XCTAssertTrue(text(containing: "reading the diff").waitForExistence(timeout: 20), "an agent's thinking status shows; on screen: \(visibleText())")
        let disclosure = app.buttons["Show full response"].firstMatch
        XCTAssertTrue(disclosure.waitForExistence(timeout: 30), "a long message is collapsed; on screen: \(visibleText())")
        XCTAssertFalse(text(containing: "last line of the long report").exists, "the collapsed message hides its tail")

        XCTAssertTrue(app.staticTexts["Latency by hour"].waitForExistence(timeout: 75), "a chart message draws its title and picture; on screen: \(visibleText())")
        XCTAssertTrue(app.staticTexts["cowchat #12"].waitForExistence(timeout: 30), "a pull request row shows; on screen: \(visibleText())")
        XCTAssertTrue(app.staticTexts["Checks passed"].exists, "the row shows its checks")

        app.buttons["room.more"].tap()
        app.buttons["People"].tap()
        XCTAssertTrue(app.staticTexts["ada-agent"].waitForExistence(timeout: 10), "People lists a connected agent; on screen: \(visibleText())")
        app.buttons["people.done"].tap()

        app.buttons["room.more"].tap()
        app.buttons["Share invite"].tap()
        let closeShare = app.navigationBars["UIActivityContentView"].buttons["Close"]
        XCTAssertTrue(
            waitFor(timeout: 10) { closeShare.exists || self.app.otherElements["ActivityListView"].exists },
            "Share invite opens the share sheet; on screen: \(visibleText())"
        )
        if closeShare.exists { closeShare.tap() } else { app.swipeDown(velocity: .fast) }
        XCTAssertTrue(waitFor(timeout: 5) { self.app.buttons["room.more"].isHittable }, "the share sheet closes")

        let seenBefore = pingLabels()
        XCUIDevice.shared.press(.home)
        sleep(8)
        app.activate()
        XCTAssertTrue(waitFor(timeout: 30) { !self.pingLabels().subtracting(seenBefore).isEmpty }, "new messages after returning from background")

        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.staticTexts["sealed"].waitForExistence(timeout: 5))
        let unread = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Unread")).firstMatch
        XCTAssertTrue(unread.waitForExistence(timeout: 20), "closed room goes unread on a live message")

        press(app.staticTexts["sealed"])
        XCTAssertTrue(text(containing: "Encrypted message").waitForExistence(timeout: 10), "locked before the secret")
        app.buttons["Enter room secret"].firstMatch.tap()
        type("correct horse battery", into: app.secureTextFields["Room secret"])
        app.buttons["Save"].tap()
        // The seeded message, or a reply an earlier run left once those fill the screen.
        XCTAssertTrue(
            waitFor(timeout: 10) { self.text(containing: "the sealed plan is ready").exists || self.text(containing: "sealed reply").exists },
            "decrypts with the secret; on screen: \(visibleText())"
        )

        let sealedComposer = app.textFields["composer.field"].exists ? app.textFields["composer.field"] : app.textViews["composer.field"]
        sealedComposer.tap()
        sealedComposer.typeText("sealed reply \(nonce)")
        app.buttons["composer.send"].tap()
        XCTAssertTrue(text(containing: "sealed reply \(nonce)").waitForExistence(timeout: 10))

        app.buttons["room.more"].tap()
        app.buttons["Archive room"].tap()
        let archived = app.buttons["rooms.archived"]
        // The Archived row is last, so a long list has it below the fold.
        if !archived.waitForExistence(timeout: 3) { app.swipeUp() }
        XCTAssertTrue(archived.waitForExistence(timeout: 5), "archiving returns to the list and shows Archived; on screen: \(visibleText())")
        XCTAssertFalse(app.staticTexts["sealed"].exists, "the archived room leaves the main list")
        archived.tap()
        let sealedRow = app.staticTexts["sealed"]
        XCTAssertTrue(sealedRow.waitForExistence(timeout: 5), "Archived expands to show the room")
        // Rooms left by earlier runs make the list long, so the expanded row can sit under the bottom edge.
        if !sealedRow.isHittable || sealedRow.frame.maxY > app.frame.maxY - 120 { app.swipeUp() }
        swipeLeft(on: sealedRow)
        let unarchive = app.buttons["Unarchive"].firstMatch
        // A drag that starts while the list is still settling is taken as a scroll.
        if !unarchive.waitForExistence(timeout: 3) { swipeLeft(on: sealedRow) }
        XCTAssertTrue(unarchive.waitForExistence(timeout: 5), "swipe shows Unarchive")
        unarchive.tap()
        XCTAssertTrue(waitFor(timeout: 5) { !archived.exists && sealedRow.exists }, "the room returns to the main list")

        let search = app.searchFields["Search"]
        XCTAssertTrue(search.waitForExistence(timeout: 5), "the room list has a search field")
        search.tap()
        search.typeText("phone \(nonce)")
        XCTAssertTrue(app.staticTexts["design-review"].waitForExistence(timeout: 15), "search finds a room by message text; on screen: \(visibleText())")
        XCTAssertFalse(app.staticTexts["sealed"].exists, "rooms that do not match are left out")
        let leave = ["Cancel", "Close", "close"].map { app.buttons[$0].firstMatch }.first(where: \.exists)
        XCTAssertNotNil(leave, "a way out of search; buttons: \(app.buttons.allElementsBoundByIndex.map { "\($0.identifier)/\($0.label)" })")
        leave?.tap()
        XCTAssertTrue(app.staticTexts["sealed"].waitForExistence(timeout: 5), "cancelling search restores the list")

        let made = "phone-\(nonce)"
        chooseFromAddMenu("New room")
        type(made, into: app.textFields["newRoom.name"])
        app.buttons["newRoom.create"].tap()
        XCTAssertTrue(app.navigationBars[made].waitForExistence(timeout: 10), "the new room opens; on screen: \(visibleText())")
        let madeComposer = app.textFields["composer.field"].exists ? app.textFields["composer.field"] : app.textViews["composer.field"]
        madeComposer.tap()
        madeComposer.typeText("first post \(nonce)")
        app.buttons["composer.send"].tap()
        XCTAssertTrue(text(containing: "first post \(nonce)").waitForExistence(timeout: 10))
        app.buttons["room.more"].tap()
        app.buttons["Rename room"].tap()
        let renameField = app.alerts.textFields.firstMatch
        XCTAssertTrue(renameField.waitForExistence(timeout: 5), "Rename shows for a room the phone made")
        renameField.typeText("-2")
        app.alerts.buttons["Rename"].tap()
        XCTAssertTrue(app.navigationBars[made + "-2"].waitForExistence(timeout: 10), "the room takes its new name; on screen: \(visibleText())")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.staticTexts[made + "-2"].waitForExistence(timeout: 5), "the renamed room is in the list")
        // A long list keeps its last rows off screen, where the test cannot see them.
        if !app.staticTexts["late-\(nonce)"].waitForExistence(timeout: 40) { app.swipeUp() }
        XCTAssertTrue(app.staticTexts["late-\(nonce)"].waitForExistence(timeout: 30), "a room made elsewhere appears without a refresh; on screen: \(visibleText())")

        guard let invite = env["COWCHAT_TEST_INVITE"], !invite.isEmpty else { return }
        chooseFromAddMenu("Add connection")
        fillOwnServer(port: port)
        type(invite, into: app.secureTextFields["signin.paste"])
        XCTAssertTrue(app.staticTexts["Invite"].waitForExistence(timeout: 5), "the paste is recognised as an invite")
        app.buttons["signin.connect"].tap()
        let invitedRow = app.staticTexts["invited"]
        XCTAssertTrue(invitedRow.waitForExistence(timeout: 10), "the invite joins a room on the existing connection; on screen: \(visibleText())")
        XCTAssertFalse(app.buttons["signin.connect"].exists, "the sheet closes")
        // Rooms left by earlier runs make the list long, and the tab bar covers its last rows.
        if invitedRow.frame.maxY > app.frame.maxY - 170 { app.swipeUp() }
        swipeLeft(on: invitedRow)
        let remove = app.buttons["Remove"].firstMatch
        if !remove.waitForExistence(timeout: 3) { swipeLeft(on: invitedRow) }
        XCTAssertTrue(remove.waitForExistence(timeout: 5), "swipe shows Remove; buttons: \(app.buttons.allElementsBoundByIndex.map { "\($0.identifier)/\($0.label)" })")
        remove.tap()
        app.buttons["rooms.confirmRemove"].firstMatch.tap()
        XCTAssertTrue(waitFor(timeout: 5) { !invitedRow.exists }, "the removed room leaves the list")
    }

    private func fillOwnServer(port: String) {
        app.buttons["signin.ownServer"].tap()
        type("127.0.0.1", into: app.textFields["signin.host"])
        let portField = app.textFields["signin.port"]
        portField.tap()
        portField.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 5) + port)
    }

    private func signOutIfNeeded() {
        #if !targetEnvironment(simulator)
        // On a real device this would remove the person's own connections and sign their account out.
        XCTFail("this test removes every connection, so it only runs in the Simulator")
        return
        #else
        if app.buttons["signin.ownServer"].waitForExistence(timeout: 8) { return }
        openConnections()
        while app.buttons["connections.row"].firstMatch.waitForExistence(timeout: 3) {
            app.buttons["connections.row"].firstMatch.tap()
            app.buttons["connections.remove"].firstMatch.tap()
            app.buttons["connections.confirmRemove"].firstMatch.tap()
        }
        XCTAssertTrue(app.buttons["signin.ownServer"].waitForExistence(timeout: 5), "signed out; on screen: \(visibleText()); buttons: \(app.buttons.allElementsBoundByIndex.map(\.identifier))")
        #endif
    }

    // An iPhone draws the tabs in a bar at the bottom; an iPad draws them as buttons at the top.
    private func tab(_ name: String) -> XCUIElement {
        let inBar = app.tabBars.buttons[name]
        return inBar.exists ? inBar : app.buttons[name].firstMatch
    }

    private func openTab(_ name: String) {
        XCTAssertTrue(waitFor(timeout: 10) { self.tab(name).exists }, "the \(name) tab; buttons: \(app.buttons.allElementsBoundByIndex.map(\.label).prefix(12))")
        tab(name).tap()
    }

    // Connections are rows on the You tab.
    private func openConnections() {
        openTab("You")
    }

    // A sheet that is still closing swallows the tap that opens the menu, so try again.
    private func chooseFromAddMenu(_ item: String) {
        openTab("Rooms")
        for _ in 0..<4 {
            if !app.buttons[item].exists { app.buttons["rooms.add"].tap() }
            if app.buttons[item].waitForExistence(timeout: 3) { break }
        }
        app.buttons[item].tap()
    }

    // `swipeLeft()` on a row's text acts as a tap, so drag by coordinate.
    private func swipeLeft(on row: XCUIElement) {
        let start = row.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0.5))
        start.withOffset(CGVector(dx: 220, dy: 0))
            .press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: -20, dy: 0)))
    }

    private func type(_ value: String, into field: XCUIElement) {
        XCTAssertTrue(field.waitForExistence(timeout: 5), "\(field)")
        field.tap()
        // A tap that lands while a sheet is still settling does not focus the field.
        if !waitFor(timeout: 2, { (field.value(forKey: "hasKeyboardFocus") as? Bool) == true }) { field.tap() }
        field.typeText(value)
    }

    private func text(containing fragment: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", fragment)).firstMatch
    }

    private func press(_ element: XCUIElement) {
        element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    }

    private func visibleText() -> String {
        app.staticTexts.allElementsBoundByIndex.map(\.label).joined(separator: " | ")
    }

    private func pingLabels() -> Set<String> {
        // One snapshot of the whole screen: reading elements one by one races with new messages arriving.
        guard let root = try? app.snapshot() else { return [] }
        var labels: Set<String> = []
        var pending: [XCUIElementSnapshot] = [root]
        while let next = pending.popLast() {
            if next.label.contains("live ping") { labels.insert(next.label) }
            pending += next.children
        }
        return labels
    }

    private func waitFor(timeout: TimeInterval, _ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition() { return true }
            sleep(1)
        }
        return false
    }
}
