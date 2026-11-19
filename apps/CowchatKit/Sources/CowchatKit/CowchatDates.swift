import Foundation

extension Room {
    public var activityDate: Date? {
        (lastActivity ?? createdAt).cowchatDate
    }
}

extension String {
    public var cowchatDate: Date? {
        guard !isEmpty else { return nil }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return fractional.date(from: self) ?? plain.date(from: self)
    }

    public var cowchatRelativeTime: String {
        cowchatRelativeTime(relativeTo: Date())
    }

    /// The cell's short time for a timestamp from the server (`Date.cowchatShortTime(relativeTo:calendar:)`).
    public func cowchatShortTime(relativeTo now: Date, calendar: Calendar = .current) -> String {
        cowchatDate?.cowchatShortTime(relativeTo: now, calendar: calendar) ?? ""
    }

    public func cowchatRelativeTime(relativeTo now: Date) -> String {
        guard let date = cowchatDate else { return "" }
        // Callers pass a TimelineView tick date, captured when the view was
        // last evaluated, so an event that just arrived can equal or exceed
        // it. RelativeDateTimeFormatter renders any delta that rounds to zero
        // as "in 0s" — a future reading for something that just happened.
        guard now.timeIntervalSince(date) >= 1 else { return "now" }
        let formatter = RelativeDateTimeFormatter()
        formatter.dateTimeStyle = .numeric
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: date, relativeTo: now)
    }
}

extension Date {
    // copy:
    /// The sidebar cell's time (navigation note, section 3.4): "now" under a minute, "9m" under an hour, "3h" under a
    /// day, "2d" under a week, then the day and month ("Sep 3"). A time at or after `now` reads "now".
    public func cowchatShortTime(relativeTo now: Date, calendar: Calendar = .current) -> String {
        let seconds = now.timeIntervalSince(self)
        if seconds < 60 { return "now" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        if seconds < 86_400 { return "\(Int(seconds / 3600))h" }
        if seconds < 7 * 86_400 { return "\(Int(seconds / 86_400))d" }
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "MMM d"
        return formatter.string(from: self)
    }
}
