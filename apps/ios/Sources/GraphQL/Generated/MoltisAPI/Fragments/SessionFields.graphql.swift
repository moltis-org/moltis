// @generated
// This file was automatically generated and should not be edited.

@_exported import ApolloAPI
@_spi(Execution) @_spi(Unsafe) import ApolloAPI

extension MoltisAPI {
  struct SessionFields: MoltisAPI.SelectionSet, Fragment {
    static var fragmentDefinition: StaticString {
      #"fragment SessionFields on SessionEntry { __typename id key label model preview createdAt updatedAt messageCount lastSeenMessageCount archived }"#
    }

    let __data: DataDict
    init(_dataDict: DataDict) { __data = _dataDict }

    static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.SessionEntry }
    static var __selections: [ApolloAPI.Selection] { [
      .field("__typename", String.self),
      .field("id", String?.self),
      .field("key", String?.self),
      .field("label", String?.self),
      .field("model", String?.self),
      .field("preview", String?.self),
      .field("createdAt", Int?.self),
      .field("updatedAt", Int?.self),
      .field("messageCount", Int?.self),
      .field("lastSeenMessageCount", Int?.self),
      .field("archived", Bool?.self),
    ] }
    static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
      SessionFields.self
    ] }

    var id: String? { __data["id"] }
    var key: String? { __data["key"] }
    var label: String? { __data["label"] }
    var model: String? { __data["model"] }
    var preview: String? { __data["preview"] }
    var createdAt: Int? { __data["createdAt"] }
    var updatedAt: Int? { __data["updatedAt"] }
    var messageCount: Int? { __data["messageCount"] }
    var lastSeenMessageCount: Int? { __data["lastSeenMessageCount"] }
    var archived: Bool? { __data["archived"] }
  }

}