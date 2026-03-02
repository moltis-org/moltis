// @generated
// This file was automatically generated and should not be edited.

@_exported import ApolloAPI
@_spi(Execution) @_spi(Unsafe) import ApolloAPI

extension MoltisAPI {
  struct SearchSessionsQuery: GraphQLQuery {
    static let operationName: String = "SearchSessions"
    static let operationDocument: ApolloAPI.OperationDocument = .init(
      definition: .init(
        #"query SearchSessions($query: String!) { sessions { __typename search(query: $query) { __typename ...SessionFields } } }"#,
        fragments: [SessionFields.self]
      ))

    public var query: String

    public init(query: String) {
      self.query = query
    }

    @_spi(Unsafe) public var __variables: Variables? { ["query": query] }

    struct Data: MoltisAPI.SelectionSet {
      let __data: DataDict
      init(_dataDict: DataDict) { __data = _dataDict }

      static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.QueryRoot }
      static var __selections: [ApolloAPI.Selection] { [
        .field("sessions", Sessions.self),
      ] }
      static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
        SearchSessionsQuery.Data.self
      ] }

      /// Session queries.
      var sessions: Sessions { __data["sessions"] }

      /// Sessions
      ///
      /// Parent Type: `SessionQuery`
      struct Sessions: MoltisAPI.SelectionSet {
        let __data: DataDict
        init(_dataDict: DataDict) { __data = _dataDict }

        static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.SessionQuery }
        static var __selections: [ApolloAPI.Selection] { [
          .field("__typename", String.self),
          .field("search", [Search].self, arguments: ["query": .variable("query")]),
        ] }
        static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
          SearchSessionsQuery.Data.Sessions.self
        ] }

        /// Search sessions by query.
        var search: [Search] { __data["search"] }

        /// Sessions.Search
        ///
        /// Parent Type: `SessionEntry`
        struct Search: MoltisAPI.SelectionSet {
          let __data: DataDict
          init(_dataDict: DataDict) { __data = _dataDict }

          static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.SessionEntry }
          static var __selections: [ApolloAPI.Selection] { [
            .field("__typename", String.self),
            .fragment(SessionFields.self),
          ] }
          static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
            SearchSessionsQuery.Data.Sessions.Search.self,
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

          struct Fragments: FragmentContainer {
            let __data: DataDict
            init(_dataDict: DataDict) { __data = _dataDict }

            var sessionFields: SessionFields { _toFragment() }
          }
        }
      }
    }
  }

}