// @generated
// This file was automatically generated and should not be edited.

@_exported import ApolloAPI
@_spi(Execution) @_spi(Unsafe) import ApolloAPI

extension MoltisAPI {
  struct FetchSessionsQuery: GraphQLQuery {
    static let operationName: String = "FetchSessions"
    static let operationDocument: ApolloAPI.OperationDocument = .init(
      definition: .init(
        #"query FetchSessions { sessions { __typename list { __typename ...SessionFields } } }"#,
        fragments: [SessionFields.self]
      ))

    public init() {}

    struct Data: MoltisAPI.SelectionSet {
      let __data: DataDict
      init(_dataDict: DataDict) { __data = _dataDict }

      static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.QueryRoot }
      static var __selections: [ApolloAPI.Selection] { [
        .field("sessions", Sessions.self),
      ] }
      static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
        FetchSessionsQuery.Data.self
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
          .field("list", [List].self),
        ] }
        static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
          FetchSessionsQuery.Data.Sessions.self
        ] }

        /// List all sessions.
        var list: [List] { __data["list"] }

        /// Sessions.List
        ///
        /// Parent Type: `SessionEntry`
        struct List: MoltisAPI.SelectionSet {
          let __data: DataDict
          init(_dataDict: DataDict) { __data = _dataDict }

          static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.SessionEntry }
          static var __selections: [ApolloAPI.Selection] { [
            .field("__typename", String.self),
            .fragment(SessionFields.self),
          ] }
          static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
            FetchSessionsQuery.Data.Sessions.List.self,
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