module DashboardTest exposing (suite)

{-| Tests for Page.Dashboard, specifically the GotGitHubStatus handler
and build status event handlers.

This test verifies the fix for bugs where:
1. GotGitHubStatus calculated a status but never used it
2. Build status events (failed/success) didn't update the UI state
3. Build logs error events didn't clear actionInProgress
-}

import Effect exposing (Effect)
import Expect exposing (Expectation)
import Page.Dashboard as Dashboard
import Shared
import Test exposing (..)


{-| Create a minimal Shared.Model for testing.
WARNING: This uses Debug.todo for navKey which will crash if evaluated.
The functions we're testing (handleBuildStatusEvent, handleBuildLogsEvent)
only use shared.token, so navKey should never be evaluated in these tests.
If tests fail with "TODO navKey", it means the code path is trying to use navKey.
-}
testSharedModel : Maybe String -> Shared.Model
testSharedModel maybeToken =
    { navKey = Debug.todo "navKey - should not be used in handleBuildStatusEvent/handleBuildLogsEvent tests"
    , currentRoute = Nothing
    , serverVersion = ""
    , user = Nothing
    , token = maybeToken
    , refreshToken = Nothing
    , githubStatus = Shared.GitHubUnknown
    , sseConnectionState = "connected"
    }


{-| Create a CreateAppState for testing.
-}
testCreateAppState : String -> Dashboard.CreateAppState
testCreateAppState appName =
    { appName = appName
    , step = Dashboard.CheckingGitHub
    , error = Nothing
    }


{-| Helper to check if an effect contains UpdateGitHubStatus.
-}
effectContainsUpdateGitHubStatus : Effect msg -> Bool
effectContainsUpdateGitHubStatus effect =
    case effect of
        Effect.UpdateGitHubStatus _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsUpdateGitHubStatus effects

        _ ->
            False


{-| Helper to check if an effect contains FetchRepos.
-}
effectContainsFetchRepos : Effect msg -> Bool
effectContainsFetchRepos effect =
    case effect of
        Effect.FetchRepos _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsFetchRepos effects

        _ ->
            False


{-| Helper to check if an effect is Effect.none (or effectively empty).
-}
effectIsNone : Effect msg -> Bool
effectIsNone effect =
    case effect of
        Effect.None ->
            True

        Effect.Batch [] ->
            True

        _ ->
            False


{-| Check if a step is SelectRepo.
-}
isSelectRepoStep : Dashboard.CreateAppStep -> Bool
isSelectRepoStep step =
    case step of
        Dashboard.SelectRepo _ _ ->
            True

        _ ->
            False


{-| Check if a step is ConnectGitHub.
-}
isConnectGitHubStep : Dashboard.CreateAppStep -> Bool
isConnectGitHubStep step =
    case step of
        Dashboard.ConnectGitHub _ ->
            True

        _ ->
            False


{-| Check if a step is EnterName.
-}
isEnterNameStep : Dashboard.CreateAppStep -> Bool
isEnterNameStep step =
    case step of
        Dashboard.EnterName ->
            True

        _ ->
            False


suite : Test
suite =
    describe "Page.Dashboard"
        [ describe "handleGotGitHubStatus"
        [ describe "when GitHub is connected"
            [ test "transitions to SelectRepo step" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( newStep, _, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    isSelectRepoStep newStep
                        |> Expect.equal True
                        |> Expect.onFail "should be SelectRepo step"
            , test "clears any previous error" <|
                \_ ->
                    let
                        createState =
                            { appName = "my-app"
                            , step = Dashboard.CheckingGitHub
                            , error = Just "some previous error"
                            }

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, newError, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    Expect.equal Nothing newError
            , test "emits UpdateGitHubStatus effect" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectContainsUpdateGitHubStatus effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit UpdateGitHubStatus effect"
            , test "emits FetchRepos effect" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectContainsFetchRepos effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit FetchRepos effect"
            , test "does NOT return Effect.none (regression test for bug fix)" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectIsNone effect
                        |> Expect.equal False
                        |> Expect.onFail "should NOT return Effect.none - this was the original bug"
            ]
        , describe "when GitHub is not connected"
            [ test "transitions to ConnectGitHub step" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = False
                            , username = Nothing
                            }

                        ( newStep, _, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    isConnectGitHubStep newStep
                        |> Expect.equal True
                        |> Expect.onFail "should be ConnectGitHub step"
            , test "clears any previous error" <|
                \_ ->
                    let
                        createState =
                            { appName = "my-app"
                            , step = Dashboard.CheckingGitHub
                            , error = Just "some previous error"
                            }

                        response =
                            { connected = False
                            , username = Nothing
                            }

                        ( _, newError, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    Expect.equal Nothing newError
            , test "emits UpdateGitHubStatus effect" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = False
                            , username = Nothing
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectContainsUpdateGitHubStatus effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit UpdateGitHubStatus effect"
            , test "does NOT return Effect.none (regression test for bug fix)" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = False
                            , username = Nothing
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectIsNone effect
                        |> Expect.equal False
                        |> Expect.onFail "should NOT return Effect.none - this was the original bug"
            ]
        , describe "when GitHub status check fails"
            [ test "transitions to EnterName step" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        ( newStep, _, _ ) =
                            Dashboard.handleGotGitHubStatus (Err "Network error") createState (Just "test-token")
                    in
                    isEnterNameStep newStep
                        |> Expect.equal True
                        |> Expect.onFail "should be EnterName step"
            , test "sets error message" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        ( _, newError, _ ) =
                            Dashboard.handleGotGitHubStatus (Err "Network error") createState (Just "test-token")
                    in
                    Expect.equal (Just "Failed to check GitHub connection") newError
            ]
        , describe "when no token is available"
            [ test "keeps the current step unchanged" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( newStep, _, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState Nothing
                    in
                    -- Should stay on CheckingGitHub step since no token
                    Expect.equal Dashboard.CheckingGitHub newStep
            , test "returns Effect.none" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState Nothing
                    in
                    effectIsNone effect
                        |> Expect.equal True
                        |> Expect.onFail "should return Effect.none when no token"
            ]
        ]
        {- NOTE: The handleBuildStatusEvent and handleBuildLogsEvent tests below
           require a Shared.Model with a Browser.Navigation.Key, which is opaque
           and can't be created in unit tests. These tests use Debug.todo for navKey.
           The functions being tested only use shared.token, so navKey should never
           be evaluated. If these tests fail with "TODO navKey", it indicates the
           code path is trying to use navKey unexpectedly.
           
           TODO: Convert these to integration tests using elm-program-test which
           can provide a real navKey, or refactor to make navKey optional for testing.
        -}
        , describe "handleBuildStatusEvent"
        [ describe "when build status is 'failed'"
            [ test "updates app state to 'failed' in apps list" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            }

                        model =
                            { view = Dashboard.AppsListView
                            , apps = [ testApp ]
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "failed"
                    in
                    case updatedModel.apps of
                        [ app ] ->
                            Expect.equal "failed" app.state

                        _ ->
                            Expect.fail "Expected exactly one app in the list"
            , test "clears actionInProgress when viewing app detail" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Just Dashboard.Building
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "failed"
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.equal Nothing updatedDetailState.actionInProgress

                        _ ->
                            Expect.fail "Expected AppDetailView"
            , test "emits FetchAppDetail and FetchBuilds effects when viewing app detail" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Just Dashboard.Building
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( _, effect ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "failed"
                    in
                    Expect.all
                        [ \_ -> effectContainsFetchAppDetail effect |> Expect.equal True
                        , \_ -> effectContainsFetchBuilds effect |> Expect.equal True
                        ]
                        ()
            , test "emits FetchApps effect when on apps list view" <|
                \_ ->
                    let
                        model =
                            { view = Dashboard.AppsListView
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( _, effect ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "failed"
                    in
                    effectContainsFetchApps effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit FetchApps effect when on apps list view"
            ]
        , describe "when build status is 'success'"
            [ test "updates app state from 'building' to 'stopped' in apps list" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            }

                        model =
                            { view = Dashboard.AppsListView
                            , apps = [ testApp ]
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "success"
                    in
                    case updatedModel.apps of
                        [ app ] ->
                            Expect.equal "stopped" app.state

                        _ ->
                            Expect.fail "Expected exactly one app in the list"
            , test "clears actionInProgress when viewing app detail" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Just Dashboard.Building
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "success"
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.equal Nothing updatedDetailState.actionInProgress

                        _ ->
                            Expect.fail "Expected AppDetailView"
            ]
        ]
    , describe "Page.Dashboard.handleBuildLogsEvent"
        [ describe "when event type is 'error'"
            [ test "clears actionInProgress" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.BuildLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Just Dashboard.Building
                            , error = Nothing
                            , streamingBuildId = Just "build-123"
                            , buildLogsStreaming = True
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildLogsEvent model shared "my-app" "build-123" "error" "Build failed"
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.all
                                [ \_ -> Expect.equal Nothing updatedDetailState.actionInProgress
                                , \_ -> Expect.equal False updatedDetailState.buildLogsStreaming
                                , \_ -> Expect.equal Nothing updatedDetailState.streamingBuildId
                                , \_ -> Expect.equal (Just "Build error: Build failed") updatedDetailState.error
                                ]
                                ()

                        _ ->
                            Expect.fail "Expected AppDetailView"
            , test "emits FetchAppDetail and FetchBuilds effects" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.BuildLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Just Dashboard.Building
                            , error = Nothing
                            , streamingBuildId = Just "build-123"
                            , buildLogsStreaming = True
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( _, effect ) =
                            Dashboard.handleBuildLogsEvent model shared "my-app" "build-123" "error" "Build failed"
                    in
                    Expect.all
                        [ \_ -> effectContainsFetchAppDetail effect |> Expect.equal True
                        , \_ -> effectContainsFetchBuilds effect |> Expect.equal True
                        ]
                        ()
            ]
        ]
    ]


{-| Helper to check if an effect contains FetchAppDetail.
-}
effectContainsFetchAppDetail : Effect msg -> Bool
effectContainsFetchAppDetail effect =
    case effect of
        Effect.FetchAppDetail _ _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsFetchAppDetail effects

        _ ->
            False


{-| Helper to check if an effect contains FetchBuilds.
-}
effectContainsFetchBuilds : Effect msg -> Bool
effectContainsFetchBuilds effect =
    case effect of
        Effect.FetchBuilds _ _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsFetchBuilds effects

        _ ->
            False


{-| Helper to check if an effect contains FetchApps.
-}
effectContainsFetchApps : Effect msg -> Bool
effectContainsFetchApps effect =
    case effect of
        Effect.FetchApps _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsFetchApps effects

        _ ->
            False
