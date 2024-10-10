import Cocoa
import ApplicationServices
import Foundation

class QueueElement {
    let element: AXUIElement
    let depth: Int
    
    init(_ element: AXUIElement, depth: Int) {
        self.element = element
        self.depth = depth
    }
}

func printAllAttributeValues(_ startElement: AXUIElement, to fileHandle: FileHandle?) {
    var elements: [(CGPoint, CGSize, String)] = []
    var visitedElements = Set<AXUIElement>()
    let unwantedValues = ["0", "", "", "3", ""]
    let unwantedLabels = [
        "window", "application", "group", "button", "image", "text",
        "pop up button", "region", "notifications", "table", "column",
        "html content"
    ]
    
    func traverseHierarchy(_ element: AXUIElement, depth: Int) {
        guard !visitedElements.contains(element) else { return }
        visitedElements.insert(element)
        
        var attributeNames: CFArray?
        let result = AXUIElementCopyAttributeNames(element, &attributeNames)
        
        guard result == .success, let attributes = attributeNames as? [String] else { return }
        
        var position: CGPoint = .zero
        var size: CGSize = .zero
        
        // Get position
        if let positionValue = getAttributeValue(element, forAttribute: kAXPositionAttribute) as! AXValue?,
           AXValueGetType(positionValue) == .cgPoint {
            AXValueGetValue(positionValue, .cgPoint, &position)
        }
        
        // Get size
        if let sizeValue = getAttributeValue(element, forAttribute: kAXSizeAttribute) as! AXValue?,
           AXValueGetType(sizeValue) == .cgSize {
            AXValueGetValue(sizeValue, .cgSize, &size)
        }
        
        for attr in attributes {
            if ["AXDescription", "AXValue", "AXLabel", "AXRoleDescription", "AXHelp"].contains(attr) {
                if let value = getAttributeValue(element, forAttribute: attr) {
                    let valueStr = describeValue(value)
                    if !valueStr.isEmpty && !unwantedValues.contains(valueStr) && valueStr.count > 1 &&
                       !unwantedLabels.contains(valueStr.lowercased()) {
                        elements.append((position, size, valueStr))
                    }
                }
            }
            
            // Traverse child elements
            if let childrenValue = getAttributeValue(element, forAttribute: attr) {
                if let elementArray = childrenValue as? [AXUIElement] {
                    for childElement in elementArray {
                        traverseHierarchy(childElement, depth: depth + 1)
                    }
                } else if let childElement = childrenValue as! AXUIElement? {
                    traverseHierarchy(childElement, depth: depth + 1)
                }
            }
        }
    }
    
    traverseHierarchy(startElement, depth: 0)
    
    // Sort elements from top to bottom, then left to right
    elements.sort { (a, b) in
        if a.0.y != b.0.y {
            return a.0.y < b.0.y
        } else {
            return a.0.x < b.0.x
        }
    }
    
    // Print sorted elements
    for (position, size, valueStr) in elements {
        let coordinates = formatCoordinates(position, size)
        let output = "\(coordinates) \(valueStr)\n"
        print(output, terminator: "")
        fileHandle?.write(output.data(using: .utf8)!)
    }
}

func formatCoordinates(_ position: CGPoint, _ size: CGSize) -> String {
    return String(format: "(x:%.0f,y:%.0f,w:%.0f,h:%.0f)", position.x, position.y, size.width, size.height)
}

func describeValue(_ value: AnyObject?) -> String {
    switch value {
    case let string as String:
        return string
    case let number as NSNumber:
        return number.stringValue
    case let point as NSPoint:
        return "(\(point.x), \(point.y))"
    case let size as NSSize:
        return "w=\(size.width) h=\(size.height)"
    case let rect as NSRect:
        return "x=\(rect.origin.x) y=\(rect.origin.y) w=\(rect.size.width) h=\(rect.size.height)"
    case let range as NSRange:
        return "loc=\(range.location) len=\(range.length)"
    case let url as URL:
        return url.absoluteString
    case let array as [AnyObject]:
        return array.isEmpty ? "Empty array" : "Array with \(array.count) elements"
    case let axValue as AXValue:
        return describeAXValue(axValue)
    case is AXUIElement:
        return "AXUIElement"
    case .none:
        return "None"
    default:
        return String(describing: value)
    }
}

func describeAXValue(_ axValue: AXValue) -> String {
    let type = AXValueGetType(axValue)
    switch type {
    case .cgPoint:
        var point = CGPoint.zero
        AXValueGetValue(axValue, .cgPoint, &point)
        return "(\(point.x), \(point.y))"
    case .cgSize:
        var size = CGSize.zero
        AXValueGetValue(axValue, .cgSize, &size)
        return "w=\(size.width) h=\(size.height)"
    case .cgRect:
        var rect = CGRect.zero
        AXValueGetValue(axValue, .cgRect, &rect)
        return "x=\(rect.origin.x) y=\(rect.origin.y) w=\(rect.size.width) h=\(rect.size.height)"
    case .cfRange:
        var range = CFRange(location: 0, length: 0)
        AXValueGetValue(axValue, .cfRange, &range)
        return "loc=\(range.location) len=\(range.length)"
    default:
        return "Unknown AXValue type"
    }
}

func getAttributeValue(_ element: AXUIElement, forAttribute attr: String) -> AnyObject? {
    var value: AnyObject?
    let result = AXUIElementCopyAttributeValue(element, attr as CFString, &value)
    return result == .success ? value : nil
}

func printAllAttributeValuesForCurrentApp() {
    guard let app = NSWorkspace.shared.frontmostApplication else {
        print("couldn't get frontmost application")
        return
    }
    
    let pid = app.processIdentifier
    let axApp = AXUIElementCreateApplication(pid)
    
    let fileName = "accessibility_attributes.txt"
    let fileManager = FileManager.default
    let currentPath = fileManager.currentDirectoryPath
    let outputPath = (currentPath as NSString).appendingPathComponent(fileName)
    
    guard fileManager.createFile(atPath: outputPath, contents: nil, attributes: nil) else {
        print("couldn't create file")
        return
    }
    
    guard let fileHandle = FileHandle(forWritingAtPath: outputPath) else {
        print("couldn't open file for writing")
        return
    }
    defer {
        fileHandle.closeFile()
    }
    
    let header = "attribute values for \(app.localizedName ?? "unknown app"):\n"
    print(header, terminator: "")
    fileHandle.write(header.data(using: .utf8)!)
    
    printAllAttributeValues(axApp, to: fileHandle)
    
    print("output written to \(outputPath) and printed to terminal")
}

// usage
printAllAttributeValuesForCurrentApp()