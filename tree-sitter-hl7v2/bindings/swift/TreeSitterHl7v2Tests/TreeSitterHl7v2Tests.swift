import XCTest
import SwiftTreeSitter
import TreeSitterHl7v2

final class TreeSitterHl7v2Tests: XCTestCase {
    func testCanLoadGrammar() throws {
        let parser = Parser()
        let language = Language(language: tree_sitter_hl7v2())
        XCTAssertNoThrow(try parser.setLanguage(language),
                         "Error loading Hl7v2 grammar")
    }
}
