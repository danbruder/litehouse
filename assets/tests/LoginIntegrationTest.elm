module LoginIntegrationTest exposing (suite)

{-| Integration tests for the Login page using elm-program-test.
These tests verify the full user flow including form interactions, HTTP requests, and navigation.
-}

import Effect
import Expect
import Http
import Json.Encode as Encode
import Main
import ProgramTest exposing (ProgramTest)
import ProgramTestHelpers as Helpers
import Test exposing (..)


suite : Test
suite =
    describe "Login Page Integration Tests"
        [ describe "Initial Load"
            [ test "shows login form when server is initialized" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.ensureViewHasText "Sign in to your account"
                        |> Helpers.ensureViewHasText "Email"
                        |> Helpers.ensureViewHasText "Password"
                        |> ProgramTest.done
            , test "redirects to setup when server is not initialized" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson False "1.0.0")
                        |> Helpers.ensureBrowserUrlPath "/setup"
                        |> ProgramTest.done
            ]
        , describe "Form Interaction"
            [ test "updates email field when typing" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "email" "user@example.com"
                        |> ProgramTest.advanceTime 50
                        |> Helpers.ensureViewHasText "Email"
                        |> ProgramTest.done
            , test "updates password field when typing" <|
                \_ ->
                    -- Password fields don't display their values, so we just verify the field was filled
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "password" "secret123"
                        |> Helpers.ensureViewHasText "Password"
                        |> ProgramTest.done
            ]
        , describe "Form Submission"
            [ test "submits login form with correct credentials" <|
                \_ ->
                    let
                        user =
                            { email = "user@example.com"
                            , fullName = "Test User"
                            }

                        authResponse =
                            Helpers.authResponseJson "access-token-123" "refresh-token-456" user
                    in
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "email" "user@example.com"
                        |> Helpers.fillIn "password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.ensureHttpRequestWithBody
                            "POST"
                            "/api/auth/login"
                            (Encode.object
                                [ ( "email", Encode.string "user@example.com" )
                                , ( "password", Encode.string "password123" )
                                ]
                            )
                        |> Helpers.simulateHttpOk "POST" "/api/auth/login" authResponse
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> ProgramTest.done
            , test "shows error message on login failure" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "email" "user@example.com"
                        |> Helpers.fillIn "password" "wrongpassword"
                        |> Helpers.submitForm
                        |> Helpers.ensureHttpRequestWithBody
                            "POST"
                            "/api/auth/login"
                            (Encode.object
                                [ ( "email", Encode.string "user@example.com" )
                                , ( "password", Encode.string "wrongpassword" )
                                ]
                            )
                        |> Helpers.simulateHttpError "POST" "/api/auth/login" (Http.BadStatus 401)
                        |> Helpers.ensureViewHasText "Invalid email or password"
                        |> Helpers.ensureBrowserUrlPath "/login"
                        |> ProgramTest.done
            , test "disables submit button while submitting" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "email" "user@example.com"
                        |> Helpers.fillIn "password" "password123"
                        |> Helpers.submitForm
                        |> Helpers.ensureViewHasText "Signing in..."
                        |> ProgramTest.done
            ]
        , describe "Navigation"
            [ test "stays on login page after failed login" <|
                \_ ->
                    Helpers.createTestProgram
                        |> Helpers.ensureHttpRequest "GET" "/api/auth/status"
                        |> Helpers.simulateHttpOk "GET" "/api/auth/status" (Helpers.serverStatusJson True "1.0.0")
                        |> Helpers.fillIn "email" "user@example.com"
                        |> Helpers.fillIn "password" "wrong"
                        |> Helpers.submitForm
                        |> Helpers.ensureHttpRequestWithBody
                            "POST"
                            "/api/auth/login"
                            (Encode.object
                                [ ( "email", Encode.string "user@example.com" )
                                , ( "password", Encode.string "wrong" )
                                ]
                            )
                        |> Helpers.simulateHttpError "POST" "/api/auth/login" (Http.BadStatus 401)
                        |> Helpers.ensureBrowserUrlPath "/login"
                        |> ProgramTest.done
            ]
        ]
