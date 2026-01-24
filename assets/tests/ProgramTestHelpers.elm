module ProgramTestHelpers exposing
    ( createTestProgram
    , ensureHttpRequest
    , ensureHttpRequestWithBody
    , simulateHttpOk
    , simulateHttpError
    , expectViewHasText
    , expectViewHasNotText
    , clickButton
    , fillIn
    , submitForm
    , authResponseJson
    , serverStatusJson
    , appsListJson
    , appDetailJson
    , githubStatusJson
    , reposListJson
    )

{-| Helper functions for writing integration tests with elm-program-test.
These follow best practices for testing Elm applications.
-}

import Effect
import Expect
import Http
import Json.Encode as Encode
import Main
import ProgramTest exposing (ProgramTest)
import Test.Http
import Test.Html.Query as Query
import Test.Html.Selector exposing (text)


{-| Create a ProgramTest instance for the application with default flags.
Note: This requires the app to use withSimulatedEffects, but since Main uses Cmd directly,
we'll need to handle HTTP simulation differently. For now, this is a basic setup.
-}
createTestProgram : ProgramTest Main.Model Main.Msg (Cmd Main.Msg)
createTestProgram =
    let
        flags =
            { token = Nothing }
    in
    ProgramTest.createApplication
        { init = Main.init
        , update = Main.update
        , view = Main.view
        , onUrlChange = \url -> Main.UrlChanged url
        , onUrlRequest = \request -> Main.LinkClicked request
        }
        |> ProgramTest.withBaseUrl "http://localhost"
        |> ProgramTest.start flags


{-| Ensure that an HTTP request was made with the given method and URL.
Returns ProgramTest so it can be chained.
-}
ensureHttpRequest : String -> String -> ProgramTest model msg effect -> ProgramTest model msg effect
ensureHttpRequest method url programTest =
    ProgramTest.ensureHttpRequestWasMade method url programTest


{-| Ensure that an HTTP request was made with the given method, URL, and JSON body.
-}
ensureHttpRequestWithBody : String -> String -> Encode.Value -> ProgramTest model msg effect -> ProgramTest model msg effect
ensureHttpRequestWithBody method url expectedBody programTest =
    ProgramTest.ensureHttpRequest method
        url
        (\request ->
            request.body
                |> Expect.equal (Encode.encode 0 expectedBody)
        )
        programTest


{-| Simulate an HTTP response with status 200 and JSON body.
-}
simulateHttpOk : String -> String -> Encode.Value -> ProgramTest model msg effect -> ProgramTest model msg effect
simulateHttpOk method url responseBody programTest =
    ProgramTest.simulateHttpOk method
        url
        (Encode.encode 0 responseBody)
        programTest


{-| Simulate an HTTP error response.
-}
simulateHttpError : String -> String -> Http.Error -> ProgramTest model msg effect -> ProgramTest model msg effect
simulateHttpError method url error programTest =
    case error of
        Http.BadStatus statusCode ->
            ProgramTest.simulateHttpResponse method
                url
                { statusCode = statusCode
                , headers = []
                , body = ""
                }
                programTest

        Http.NetworkError ->
            ProgramTest.simulateHttpResponse method
                url
                { statusCode = 0
                , headers = []
                , body = ""
                }
                programTest

        _ ->
            ProgramTest.simulateHttpResponse method
                url
                { statusCode = 500
                , headers = []
                , body = ""
                }
                programTest


{-| Expect that the view contains the given text.
-}
expectViewHasText : String -> ProgramTest model msg effect -> ProgramTest model msg effect
expectViewHasText expectedText programTest =
    ProgramTest.expectView
        (Query.has [ text expectedText ])
        programTest


{-| Expect that the view does not contain the given text.
-}
expectViewHasNotText : String -> ProgramTest model msg effect -> ProgramTest model msg effect
expectViewHasNotText unexpectedText programTest =
    ProgramTest.expectView
        (Query.hasNot [ text unexpectedText ])
        programTest


{-| Click a button with the given text.
-}
clickButton : String -> ProgramTest model msg effect -> ProgramTest model msg effect
clickButton buttonText programTest =
    ProgramTest.clickButton buttonText programTest


{-| Fill in an input field with the given label.
-}
fillIn : String -> String -> ProgramTest model msg effect -> ProgramTest model msg effect
fillIn label value programTest =
    ProgramTest.fillIn label value programTest


{-| Submit a form by finding the form element and triggering submit.
-}
submitForm : ProgramTest model msg effect -> ProgramTest model msg effect
submitForm programTest =
    ProgramTest.within
        "form"
        []
        (ProgramTest.simulateDomEvent "submit" (Encode.object []))
        programTest


{-| Helper to create a JSON response for auth responses.
-}
authResponseJson : String -> String -> Effect.UserInfo -> Encode.Value
authResponseJson accessToken refreshToken user =
    Encode.object
        [ ( "tokens"
          , Encode.object
                [ ( "access_token", Encode.string accessToken )
                , ( "refresh_token", Encode.string refreshToken )
                ]
          )
        , ( "user"
          , Encode.object
                [ ( "email", Encode.string user.email )
                , ( "full_name", Encode.string user.fullName )
                ]
          )
        ]


{-| Helper to create a JSON response for server status.
-}
serverStatusJson : Bool -> String -> Encode.Value
serverStatusJson initialized version =
    Encode.object
        [ ( "initialized", Encode.bool initialized )
        , ( "version", Encode.string version )
        ]


{-| Helper to create a JSON response for apps list.
-}
appsListJson : List Effect.AppInfo -> Encode.Value
appsListJson apps =
    Encode.list
        (\app ->
            Encode.object
                [ ( "id", Encode.string app.id )
                , ( "name", Encode.string app.name )
                , ( "state", Encode.string app.state )
                ]
        )
        apps


{-| Helper to create a JSON response for app detail.
-}
appDetailJson : Effect.AppDetail -> Encode.Value
appDetailJson app =
    Encode.object
        [ ( "id", Encode.string app.id )
        , ( "name", Encode.string app.name )
        , ( "state", Encode.string app.state )
        , ( "port"
          , case app.port_ of
                Just port_ ->
                    Encode.int port_

                Nothing ->
                    Encode.null
          )
        , ( "created_at", Encode.string app.createdAt )
        , ( "updated_at", Encode.string app.updatedAt )
        , ( "remote"
          , case app.remote of
                Just remote ->
                    Encode.object
                        [ ( "name", Encode.string remote.name )
                        , ( "url", Encode.string remote.url )
                        , ( "branch", Encode.string remote.branch )
                        ]

                Nothing ->
                    Encode.null
          )
        ]


{-| Helper to create a JSON response for GitHub status.
-}
githubStatusJson : Bool -> Maybe String -> Encode.Value
githubStatusJson connected maybeUsername =
    Encode.object
        [ ( "connected", Encode.bool connected )
        , ( "username"
          , case maybeUsername of
                Just username ->
                    Encode.string username

                Nothing ->
                    Encode.null
          )
        ]


{-| Helper to create a JSON response for repos list.
-}
reposListJson : List Effect.RepoInfo -> Encode.Value
reposListJson repos =
    Encode.list
        (\repo ->
            Encode.object
                [ ( "name", Encode.string repo.name )
                , ( "full_name", Encode.string repo.fullName )
                , ( "description"
                  , case repo.description of
                        Just desc ->
                            Encode.string desc

                        Nothing ->
                            Encode.null
                  )
                , ( "private", Encode.bool repo.private )
                , ( "clone_url", Encode.string repo.cloneUrl )
                , ( "default_branch", Encode.string repo.defaultBranch )
                ]
        )
        repos
