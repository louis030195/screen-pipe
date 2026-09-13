// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

// Headless regression for the real native stream decoder and Timeline model.
import Foundation

@main
struct TimelineStreamTests {
    @MainActor
    static func main() async {
        let model = TimelineViewModel(config: TimelineAPIConfig(port: 0))
        let range = TimelineDateNavigation.dayRange(for: model.currentDate)
        precondition(model.isLoading)
        precondition(FrameStreamMessage.decode("\"keep-alive-text\"") == .keepAlive)
        let otherDay = FrameStreamCompletion(start: range.start.addingTimeInterval(-86400), end: range.end.addingTimeInterval(-86400))
        model.completeFrameRequest(otherDay)
        precondition(model.isLoading, "a stale day must not finish the selected day")
        model.completeFrameRequest(FrameStreamCompletion(start: model.currentDate, end: model.currentDate.addingTimeInterval(2)))
        precondition(model.isLoading, "a narrow search window must not finish the day")

        let json = "{\"type\":\"stream_complete\",\"start_time\":\"\(TimelineTime.iso(range.start))\",\"end_time\":\"\(TimelineTime.iso(range.end))\"}"
        guard case .complete(let completion) = FrameStreamMessage.decode(json) else {
            fatalError("server completion must decode")
        }
        model.frameStream(didComplete: completion)
        try! await Task.sleep(nanoseconds: 20_000_000)
        precondition(!model.isLoading && !model.isNavigating && model.frames.isEmpty,
                     "an empty completed day must leave the loader")
        precondition(model.connectionError == nil)

        model.changeDate(to: range.start.addingTimeInterval(-86400))
        precondition(model.isLoading && model.isNavigating)
        let previous = TimelineDateNavigation.dayRange(for: model.currentDate)
        model.completeFrameRequest(completion)
        precondition(model.isLoading, "completion of the previous request must be ignored")
        model.completeFrameRequest(FrameStreamCompletion(start: previous.start, end: previous.end, error: "Timeline request timed out"))
        precondition(!model.isLoading && !model.isNavigating)
        precondition(model.connectionError == "Timeline request timed out")
        model.stop()
        print("PASS: native stream completion, empty day, stale range, search window, navigation and error states")
    }
}
