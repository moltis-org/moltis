// @generated
// This file was automatically generated and should not be edited.

@_exported import ApolloAPI
@_spi(Execution) @_spi(Unsafe) import ApolloAPI

extension MoltisAPI {
  struct UpdateUserLocationMutation: GraphQLMutation {
    static let operationName: String = "UpdateUserLocation"
    static let operationDocument: ApolloAPI.OperationDocument = .init(
      definition: .init(
        #"mutation UpdateUserLocation($input: JSON!) { agents { __typename updateIdentity(input: $input) { __typename ok } } }"#
      ))

    public var input: JSON

    public init(input: JSON) {
      self.input = input
    }

    @_spi(Unsafe) public var __variables: Variables? { ["input": input] }

    struct Data: MoltisAPI.SelectionSet {
      let __data: DataDict
      init(_dataDict: DataDict) { __data = _dataDict }

      static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.MutationRoot }
      static var __selections: [ApolloAPI.Selection] { [
        .field("agents", Agents.self),
      ] }
      static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
        UpdateUserLocationMutation.Data.self
      ] }

      var agents: Agents { __data["agents"] }

      /// Agents
      ///
      /// Parent Type: `AgentMutation`
      struct Agents: MoltisAPI.SelectionSet {
        let __data: DataDict
        init(_dataDict: DataDict) { __data = _dataDict }

        static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.AgentMutation }
        static var __selections: [ApolloAPI.Selection] { [
          .field("__typename", String.self),
          .field("updateIdentity", UpdateIdentity.self, arguments: ["input": .variable("input")]),
        ] }
        static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
          UpdateUserLocationMutation.Data.Agents.self
        ] }

        /// Update agent identity.
        var updateIdentity: UpdateIdentity { __data["updateIdentity"] }

        /// Agents.UpdateIdentity
        ///
        /// Parent Type: `BoolResult`
        struct UpdateIdentity: MoltisAPI.SelectionSet {
          let __data: DataDict
          init(_dataDict: DataDict) { __data = _dataDict }

          static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.BoolResult }
          static var __selections: [ApolloAPI.Selection] { [
            .field("__typename", String.self),
            .field("ok", Bool.self),
          ] }
          static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
            UpdateUserLocationMutation.Data.Agents.UpdateIdentity.self
          ] }

          var ok: Bool { __data["ok"] }
        }
      }
    }
  }

}